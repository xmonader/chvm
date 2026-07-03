use std::path::{Component, Path, PathBuf};
use std::process::Command;

use tracing::info;

use crate::oci::image_config::ImageConfig;
use crate::oci::pull::PullResult;
use crate::oci::registry::sanitize_digest;

/// Container runtimes to check for local images, in priority order.
const RUNTIMES: &[&str] = &["docker", "podman"];

/// Find which local container runtime (docker/podman) has this image.
/// Returns the runtime command name if found.
pub fn find_local_runtime(image_ref: &str) -> Option<&'static str> {
    for runtime in RUNTIMES {
        let ok = Command::new(runtime)
            .args(["image", "inspect", image_ref])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return Some(runtime);
        }
    }
    None
}

/// Import an OCI image from a local container runtime using `<runtime> save`.
///
/// Both `docker save` and `podman save` produce an OCI layout tar with:
///   - index.json → manifest index
///   - blobs/sha256/<digest> → manifest, config, and layer blobs
///
/// We extract this to the chvm images directory and return a PullResult
/// compatible with the existing cache.
pub fn import_from_local(
    runtime: &str,
    image_ref: &str,
    images_dir: &Path,
) -> anyhow::Result<PullResult> {
    info!(image = %image_ref, runtime = %runtime, "Importing from local container runtime");

    let output = Command::new(runtime)
        .args(["save", image_ref])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("{runtime} save failed: {stderr}");
    }

    parse_oci_tar(&output.stdout, image_ref, images_dir)
}

/// Parse an OCI layout tar (from `docker save` / `podman save`) and import
/// layers into the chvm image cache.
fn parse_oci_tar(
    tar_data: &[u8],
    image_ref: &str,
    images_dir: &Path,
) -> anyhow::Result<PullResult> {
    // Extract the tar to a temporary directory (with path traversal protection)
    let tmp_dir = tempfile::tempdir_in(images_dir)?;
    let dest = tmp_dir.path().to_path_buf();
    safe_unpack(tar_data, &dest)?;

    // Parse index.json to find the manifest index
    let index_path = tmp_dir.path().join("index.json");
    let index: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&index_path)?)?;

    // Get the top-level manifest digest
    let top_digest = index["manifests"][0]["digest"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("no manifests in index.json"))?;

    // Read the top-level manifest (could be an image index or image manifest)
    let top_blob = read_blob(tmp_dir.path(), top_digest)?;
    let top_manifest: serde_json::Value = serde_json::from_str(&top_blob)?;

    // Find the actual image manifest — handle both image index and direct manifest
    let (manifest_digest, manifest) = if top_manifest.get("layers").is_some() {
        // Direct image manifest
        (top_digest.to_string(), top_manifest)
    } else {
        // Image index — find linux/amd64
        let manifests = top_manifest["manifests"]
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("invalid manifest index"))?;

        let entry = manifests
            .iter()
            .find(|m| {
                let p = &m["platform"];
                p["os"].as_str() == Some("linux") && p["architecture"].as_str() == Some("amd64")
            })
            .ok_or_else(|| anyhow::anyhow!("no linux/amd64 manifest found"))?;

        let digest = entry["digest"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("manifest entry has no digest"))?
            .to_string();

        let blob = read_blob(tmp_dir.path(), &digest)?;
        let m: serde_json::Value = serde_json::from_str(&blob)?;
        (digest, m)
    };

    // Parse config
    let config_digest = manifest["config"]["digest"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("manifest has no config"))?;
    let config_blob = read_blob(tmp_dir.path(), config_digest)?;
    let config_val: serde_json::Value = serde_json::from_str(&config_blob)?;
    let config = ImageConfig::from_oci_config(&config_val);

    // Create the cache directory using the manifest digest
    crate::oci::registry::validate_digest(&manifest_digest)?;
    let safe_digest = sanitize_digest(&manifest_digest);
    let image_dir = images_dir.join(&safe_digest);
    std::fs::create_dir_all(&image_dir)?;

    // Copy layer blobs to the cache directory
    let layers = manifest["layers"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("manifest has no layers"))?;

    let mut layer_paths = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let layer_digest = layer["digest"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("layer has no digest"))?;

        crate::oci::registry::validate_digest(layer_digest)?;
        let safe_layer_digest = sanitize_digest(layer_digest);
        let dest_path = image_dir.join(format!("layer-{}-{}.tar.gz", i, safe_layer_digest));

        if !dest_path.exists() {
            let src_path = blob_path(tmp_dir.path(), layer_digest);
            std::fs::copy(&src_path, &dest_path)?;
            info!(layer = i, "Copied layer");
        }

        layer_paths.push(dest_path.to_string_lossy().to_string());
    }

    drop(tmp_dir);

    info!(image = %image_ref, digest = %manifest_digest, "Local import complete");

    Ok(PullResult {
        reference: image_ref.to_string(),
        digest: manifest_digest,
        config,
        layer_paths,
        cache_dir: image_dir,
    })
}

/// Read a blob from the OCI layout by digest.
fn read_blob(oci_dir: &Path, digest: &str) -> anyhow::Result<String> {
    let path = blob_path(oci_dir, digest);
    Ok(std::fs::read_to_string(path)?)
}

/// Get the filesystem path for a blob given its digest.
fn blob_path(oci_dir: &Path, digest: &str) -> PathBuf {
    // digest format: "sha256:abc123..."
    let parts: Vec<&str> = digest.splitn(2, ':').collect();
    if parts.len() == 2 {
        oci_dir.join("blobs").join(parts[0]).join(parts[1])
    } else {
        oci_dir.join("blobs").join("sha256").join(digest)
    }
}

/// Validate that a tar entry path is safe (no path traversal).
fn sanitize_archive_path(path: &Path) -> Option<PathBuf> {
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => sanitized.push(part),
            Component::CurDir => {}
            Component::Prefix(_) | Component::RootDir | Component::ParentDir => return None,
        }
    }
    Some(sanitized)
}

/// Unpack a tar archive with path traversal protection.
/// Rejects unsafe paths and link entries that could redirect later blob reads.
fn safe_unpack(tar_data: &[u8], dest_dir: &Path) -> anyhow::Result<()> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(tar_data));
    archive.set_preserve_permissions(false);

    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = match entry.path() {
            Ok(p) => p.to_path_buf(),
            Err(_) => continue,
        };

        let Some(_safe_path) = sanitize_archive_path(&path) else {
            anyhow::bail!("unsafe tar entry path: {}", path.display());
        };

        let entry_type = entry.header().entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            anyhow::bail!("refusing link entry in OCI layout tar: {}", path.display());
        }

        match entry.unpack_in(dest_dir) {
            Ok(true) => {}
            Ok(false) => anyhow::bail!("tar entry escaped destination: {}", path.display()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
