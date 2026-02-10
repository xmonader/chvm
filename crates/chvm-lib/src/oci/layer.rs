use std::fs;
use std::path::Path;

use flate2::read::GzDecoder;
use tar::Archive;
use tracing::{debug, info, warn};

/// Extract OCI image layers to a destination directory, handling whiteouts.
pub fn extract_layers(layer_paths: &[String], dest_dir: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(dest_dir)?;

    for (i, layer_path) in layer_paths.iter().enumerate() {
        info!(layer = i, path = %layer_path, "Extracting layer");
        extract_layer(layer_path, dest_dir)?;
    }

    info!(dest = %dest_dir.display(), "Rootfs extracted");
    Ok(())
}

fn extract_layer(layer_path: &str, dest_dir: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(layer_path)?;

    // Detect compression by reading magic bytes, not file extension.
    // OCI layers may be gzip, zstd, or plain tar regardless of filename.
    let mut buf_reader = std::io::BufReader::new(file);
    let magic = {
        use std::io::Read;
        let mut buf = [0u8; 4];
        buf_reader.read_exact(&mut buf)?;
        buf
    };
    // Seek back to start
    use std::io::Seek;
    buf_reader.seek(std::io::SeekFrom::Start(0))?;

    let reader: Box<dyn std::io::Read> = if magic[0] == 0x1f && magic[1] == 0x8b {
        // Gzip magic: 1f 8b
        Box::new(GzDecoder::new(buf_reader))
    } else if magic[0..4] == [0x28, 0xb5, 0x2f, 0xfd] {
        // Zstd magic: 28 b5 2f fd
        Box::new(zstd::stream::Decoder::new(buf_reader)?)
    } else {
        // Assume plain tar
        Box::new(buf_reader)
    };

    let mut archive = Archive::new(reader);
    archive.set_preserve_permissions(true);
    archive.set_preserve_ownerships(true);
    archive.set_overwrite(true);

    for entry in archive.entries()? {
        let mut entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!(error = %e, "Skipping invalid tar entry");
                continue;
            }
        };

        let path = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(_) => continue,
        };

        let path_str = path.to_string_lossy();

        // Handle OCI whiteout files
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            if name == ".wh..wh..opq" {
                // Opaque whiteout: delete all contents of parent directory
                if let Some(parent) = path.parent() {
                    let full_parent = dest_dir.join(parent);
                    if full_parent.exists() {
                        for child in fs::read_dir(&full_parent)?.flatten() {
                            let _ = fs::remove_dir_all(child.path());
                        }
                    }
                }
                continue;
            }

            if let Some(stripped) = name.strip_prefix(".wh.") {
                // Regular whiteout: delete the specific file
                if let Some(parent) = path.parent() {
                    let target = dest_dir.join(parent).join(stripped);
                    let _ = fs::remove_file(&target);
                    let _ = fs::remove_dir_all(&target);
                }
                continue;
            }
        }

        // Skip problematic paths
        if path_str.contains("..") {
            continue;
        }

        // Extract normally
        let full_path = dest_dir.join(&path);
        if let Err(e) = entry.unpack(&full_path) {
            debug!(path = %path_str, error = %e, "Failed to extract entry (non-fatal)");
        }
    }

    Ok(())
}
