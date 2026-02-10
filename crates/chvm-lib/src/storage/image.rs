use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use tracing::{debug, info, warn};

use crate::utils::invoking_user_home;

/// Overhead multiplier for ext4 image (filesystem metadata + free space).
const IMAGE_SIZE_OVERHEAD: f64 = 1.5;
/// Minimum ext4 image size (256 MB).
const MIN_IMAGE_SIZE: u64 = 256 * 1024 * 1024;

/// Kernel modules needed for vsock and virtiofs (host-side relative paths).
/// NOTE: Keep in sync with GUEST_KERNEL_MODULES in chvm-init/src/helpers.rs
/// (the guest side uses just filenames for loading from /lib/modules/).
const KERNEL_MODULES: &[&str] = &[
    "net/vmw_vsock/vsock.ko",
    "net/vmw_vsock/vmw_vsock_virtio_transport_common.ko",
    "net/vmw_vsock/vmw_vsock_virtio_transport.ko",
    "fs/fuse/virtiofs.ko",
];

/// Supported compressed module suffixes and their decompressor commands.
const COMPRESSION_FORMATS: &[(&str, &str, &[&str])] = &[
    (".zst", "zstd", &["-d", "-f", "-o"]),
    (".xz",  "xz",   &["-d", "-k", "-f"]),
    (".gz",  "gzip",  &["-d", "-k", "-f"]),
];

/// RAII guard that unmounts a mount point on drop.
struct MountGuard {
    mount_point: std::path::PathBuf,
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        let _ = Command::new("umount").arg(&self.mount_point).output();
        let _ = std::fs::remove_dir_all(&self.mount_point);
    }
}

/// Create an ext4 block image from a rootfs directory.
pub fn create_ext4_image(
    image_path: &Path,
    rootfs_path: &Path,
    inject_mycelium: bool,
) -> anyhow::Result<()> {
    // Calculate size needed
    let size = calculate_dir_size(rootfs_path)?;
    let size = std::cmp::max((size as f64 * IMAGE_SIZE_OVERHEAD) as u64, MIN_IMAGE_SIZE);

    info!(image = %image_path.display(), size = size, "Creating ext4 block image");

    // Create sparse file
    let f = std::fs::File::create(image_path)?;
    f.set_len(size)?;
    drop(f);

    // Format as ext4
    let output = Command::new("mkfs.ext4")
        .args(["-F", "-L", "rootfs"])
        .arg(image_path)
        .output()?;
    if !output.status.success() {
        anyhow::bail!("mkfs.ext4 failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    // Mount and copy rootfs
    let mount_point = image_path.with_extension("mnt");
    std::fs::create_dir_all(&mount_point)?;

    let output = Command::new("mount")
        .args(["-o", "loop"])
        .arg(image_path)
        .arg(&mount_point)
        .output()?;
    if !output.status.success() {
        std::fs::remove_file(image_path)?;
        anyhow::bail!(
            "mount failed (try running with sudo/doas): {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    // RAII guard ensures umount + cleanup even on panic/early return
    let _guard = MountGuard { mount_point: mount_point.clone() };

    // Copy rootfs contents (cp -a preserves permissions/symlinks/devices)
    let cp_result = Command::new("cp")
        .arg("-a")
        .arg(format!("{}/.", rootfs_path.display()))
        .arg(format!("{}/", mount_point.display()))
        .output();

    if let Ok(output) = cp_result {
        if !output.status.success() {
            // _guard will unmount on drop (via the ? bail below)
            std::fs::remove_file(image_path)?;
            anyhow::bail!("cp failed: {}", String::from_utf8_lossy(&output.stderr));
        }
    }

    // Inject binaries and modules
    inject_binary(
        &mount_point,
        "chvm-init",
        "chvm-init",
        &[
            "./bin/chvm-init",
            "./target/x86_64-unknown-linux-musl/release/chvm-init",
            "./target/x86_64-unknown-linux-musl/debug/chvm-init",
            "./target/release/chvm-init",
            "./target/debug/chvm-init",
            "/usr/local/bin/chvm-init",
        ],
        Some(".chvm/bin/chvm-init"),
    );

    inject_kernel_modules(&mount_point);

    if inject_mycelium {
        inject_binary(
            &mount_point,
            "usr/bin/mycelium",
            "mycelium",
            &[
                "./bin/mycelium",
                "/usr/local/bin/mycelium",
                "/usr/bin/mycelium",
            ],
            Some(".chvm/bin/mycelium"),
        );
    }

    // _guard handles umount + cleanup on drop
    info!(image = %image_path.display(), "Block image created");
    Ok(())
}

/// Search for a binary in the given paths and inject it into the mount point.
/// `dest_rel` is relative to mount_point (e.g. "chvm-init" or "usr/bin/mycelium").
/// `home_suffix` is appended to the user's home dir (e.g. ".chvm/bin/mycelium").
pub(crate) fn inject_binary(
    mount_point: &Path,
    dest_rel: &str,
    name: &str,
    search_paths: &[&str],
    home_suffix: Option<&str>,
) {
    // Build list of candidate paths including home dir variants
    let home_path = home_suffix.and_then(|suffix| {
        invoking_user_home().map(|h| h.join(suffix))
    });

    let candidates: Vec<&Path> = search_paths.iter()
        .map(Path::new)
        .chain(home_path.as_deref())
        .collect();

    for src in candidates {
        if src.exists() {
            let dest = mount_point.join(dest_rel);
            if let Some(parent) = dest.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match std::fs::copy(src, &dest) {
                Ok(_) => {
                    let _ = std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755));
                    info!(path = %src.display(), "Injected {name}");
                    return;
                }
                Err(e) => {
                    debug!(path = %src.display(), error = %e, "Failed to copy {name}");
                }
            }
        }
    }

    warn!("{name} binary not found, skipping injection");
}

fn inject_kernel_modules(mount_point: &Path) {
    let uname = Command::new("uname").arg("-r").output().ok();
    let version = uname.map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();

    if version.is_empty() {
        debug!("Could not determine kernel version, skipping module injection");
        return;
    }

    // Check if the host kernel has modules at all. Custom-built kernels
    // with CONFIG_MODULES=n won't have a /lib/modules/<version>/ directory,
    // and all drivers are built-in -- no injection needed.
    let modules_dir = format!("/lib/modules/{version}/kernel");
    let mod_base = Path::new(&modules_dir);
    if !mod_base.exists() {
        info!(
            version = %version,
            "No kernel modules directory found (custom kernel with built-in drivers?), skipping module injection"
        );
        return;
    }

    let dest_dir = mount_point.join("lib/modules");
    let _ = std::fs::create_dir_all(&dest_dir);

    for module in KERNEL_MODULES {
        let filename = Path::new(module).file_name().unwrap().to_string_lossy();
        let dest = dest_dir.join(filename.as_ref());

        // Try uncompressed first
        let src_plain = mod_base.join(module);
        if src_plain.exists() {
            if let Err(e) = std::fs::copy(&src_plain, &dest) {
                debug!(module = %module, error = %e, "Failed to copy kernel module");
            } else {
                debug!(module = %module, "Injected kernel module");
            }
            continue;
        }

        // Try compressed variants and decompress on the host
        for &(suffix, tool, base_args) in COMPRESSION_FORMATS {
            let src = mod_base.join(format!("{module}{suffix}"));
            if src.exists() {
                let tmp_compressed = dest.with_extension(format!("ko{suffix}"));
                let _ = std::fs::copy(&src, &tmp_compressed);

                let mut cmd = Command::new(tool);
                cmd.args(base_args);
                // zstd uses "-o <dest> <src>", xz/gzip decompress in-place
                if tool == "zstd" {
                    cmd.arg(&dest).arg(&tmp_compressed);
                } else {
                    cmd.arg(&tmp_compressed);
                }
                let _ = cmd.output();

                let _ = std::fs::remove_file(&tmp_compressed);

                if dest.exists() {
                    debug!(module = %module, "Injected kernel module (decompressed from {suffix})");
                }
                break;
            }
        }
    }
}

fn calculate_dir_size(path: &Path) -> anyhow::Result<u64> {
    let mut size = 0u64;
    for entry in walkdir(path)? {
        if let Ok(meta) = std::fs::metadata(&entry) {
            if meta.is_file() {
                size += meta.len();
            }
        }
    }
    Ok(size)
}

fn walkdir(path: &Path) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    fn walk(path: &Path, files: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
        if path.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                let ft = entry.file_type()?;
                // Skip symlinks to avoid infinite loops
                if ft.is_symlink() {
                    continue;
                }
                let path = entry.path();
                if ft.is_dir() {
                    walk(&path, files)?;
                } else {
                    files.push(path);
                }
            }
        }
        Ok(())
    }
    walk(path, &mut files)?;
    Ok(files)
}
