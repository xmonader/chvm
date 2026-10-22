use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tracing::{info, warn};

use crate::storage::traits::*;

/// Per-VM qcow2 overlay disk, created on top of the shared base image.
const OVERLAY_DISK_NAME: &str = "rootfs.qcow2";
/// Per-VM full ext4 disk: the legacy (pre-overlay) layout, also used as the
/// fallback when qemu-img is unavailable.
const FULL_DISK_NAME: &str = "rootfs.img";

/// Virtio-block storage backend.
pub struct BlockStorage;

#[async_trait::async_trait]
impl StorageBackend for BlockStorage {
    fn name(&self) -> &str {
        "block"
    }

    async fn prepare(&self, config: &PrepareConfig) -> anyhow::Result<StorageResult> {
        // VMs created before overlays existed keep their writable full disk;
        // booting it preserves their data.
        let legacy_disk = config.state_dir.join(FULL_DISK_NAME);
        if legacy_disk.exists() {
            return Ok(StorageResult {
                block_device: Some(legacy_disk.to_string_lossy().to_string()),
                block_format: None,
                socket_path: None,
                virtiofs_pid: None,
                overlay_dir: None,
                overlay_backing_dir: None,
            });
        }

        let rootfs = Path::new(&config.rootfs_path);
        let base = ensure_base_image(rootfs, config.inject_mycelium, config.block_disk_size_bytes)?;
        mark_base_immutable(&base);

        if super::overlay::qemu_img().is_some() {
            let vm_disk = config.state_dir.join(OVERLAY_DISK_NAME);
            if !vm_disk.exists() {
                info!(
                    vm_id = %config.vm_id,
                    disk = %vm_disk.display(),
                    "Creating per-VM qcow2 overlay disk"
                );
                super::overlay::create_overlay(&base, &vm_disk, config.block_disk_size_bytes)?;
            } else if let Some(size) = config.block_disk_size_bytes {
                // Overlay from a previous boot: only ever grow it.
                super::overlay::resize_grow_only(&vm_disk, size)?;
            }

            return Ok(StorageResult {
                block_device: Some(vm_disk.to_string_lossy().to_string()),
                block_format: Some(super::overlay::QCOW2_FORMAT.to_string()),
                socket_path: None,
                virtiofs_pid: None,
                overlay_dir: None,
                overlay_backing_dir: None,
            });
        }

        warn!(
            vm_id = %config.vm_id,
            "qemu-img not found; falling back to a full per-VM disk copy (install qemu-img for instant qcow2 overlays)"
        );
        let vm_disk = config.state_dir.join(FULL_DISK_NAME);
        if !vm_disk.exists() {
            full_copy_disk(&base, &vm_disk)?;
        }

        Ok(StorageResult {
            block_device: Some(vm_disk.to_string_lossy().to_string()),
            block_format: None,
            socket_path: None,
            virtiofs_pid: None,
            overlay_dir: None,
            overlay_backing_dir: None,
        })
    }

    async fn cleanup(&self, vm_id: &str) -> anyhow::Result<()> {
        // Only per-VM disks inside the VM dir are removed. The shared base
        // image lives in the image cache and is never touched here.
        let vm_dir = crate::paths::Paths::resolve().vm_dir(vm_id);
        remove_vm_disks(&vm_dir)
    }
}

/// Remove the per-VM disk files from `vm_dir`. The shared base image is never
/// inside a VM dir, so it can never be reached from here.
fn remove_vm_disks(vm_dir: &Path) -> anyhow::Result<()> {
    for name in [OVERLAY_DISK_NAME, FULL_DISK_NAME] {
        let disk = vm_dir.join(name);
        if disk.exists() {
            std::fs::remove_file(&disk)?;
            info!(path = %disk.display(), "Removed per-VM disk");
        }
    }
    Ok(())
}

/// Path of the shared base ext4 image for a rootfs. For OCI images the
/// rootfs lives in the image cache at `<images>/<digest>/rootfs`, so the base
/// lands at `<images>/<digest>/rootfs.ext4.img` and is removed together with
/// the image cache entry. Local rootfs dirs get their base next to them.
fn base_image_path(rootfs_path: &Path) -> PathBuf {
    let stem = rootfs_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "rootfs".to_string());
    match rootfs_path.parent() {
        Some(parent) => parent.join(format!("{stem}.ext4.img")),
        None => PathBuf::from(format!("{stem}.ext4.img")),
    }
}

/// Build the shared base ext4 image once. Concurrent or repeated prepares
/// reuse the existing base; a build writes to a temporary name and atomically
/// renames into place so a partial base is never observed.
fn ensure_base_image(
    rootfs_path: &Path,
    inject_mycelium: bool,
    requested_size: Option<u64>,
) -> anyhow::Result<PathBuf> {
    let base = base_image_path(rootfs_path);
    if base.exists() {
        return Ok(base);
    }

    let tmp = base.with_file_name(format!(
        ".{}.tmp-{}",
        base.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "rootfs.ext4.img".to_string()),
        std::process::id()
    ));
    let _ = std::fs::remove_file(&tmp);

    super::image::create_ext4_image(&tmp, rootfs_path, inject_mycelium, requested_size)?;
    if let Err(e) = std::fs::rename(&tmp, &base) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    info!(base = %base.display(), "Created shared base ext4 image");
    Ok(base)
}

/// Bases must never be modified while overlays reference them: writes to a
/// base corrupt every overlay. Applied on every prepare; idempotent.
fn mark_base_immutable(base: &Path) {
    if let Err(e) = std::fs::set_permissions(base, std::fs::Permissions::from_mode(0o444)) {
        warn!(
            path = %base.display(),
            error = %e,
            "Failed to mark base image read-only"
        );
    }
}

/// Full-disk fallback when qemu-img is unavailable: copy the shared base into
/// the VM dir as a writable per-VM disk (the pre-overlay behavior).
fn full_copy_disk(base: &Path, dest: &Path) -> anyhow::Result<()> {
    info!(base = %base.display(), dest = %dest.display(), "Copying full disk from base image");
    std::fs::copy(base, dest)?;
    // fs::copy mirrors the base's read-only permissions; the VM needs write.
    std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o644))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_image_path_lives_next_to_image_cache_rootfs() {
        let rootfs = Path::new("/data/chvm/images/sha256-abc/rootfs");
        assert_eq!(
            base_image_path(rootfs),
            PathBuf::from("/data/chvm/images/sha256-abc/rootfs.ext4.img")
        );
    }

    #[test]
    fn base_image_path_lives_next_to_local_rootfs() {
        let rootfs = Path::new("/data/myroot");
        assert_eq!(
            base_image_path(rootfs),
            PathBuf::from("/data/myroot.ext4.img")
        );
    }

    #[test]
    fn full_copy_disk_copies_content_and_restores_writable_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("rootfs.ext4.img");
        std::fs::write(&base, b"fake ext4").unwrap();
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o444)).unwrap();

        let dest = dir.path().join("rootfs.img");
        full_copy_disk(&base, &dest).unwrap();

        assert_eq!(std::fs::read(&dest).unwrap(), b"fake ext4");
        assert_eq!(
            std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777,
            0o644
        );
    }

    /// Env-gated integration test against the real qemu-img binary:
    /// CHVM_QEMU_IMG_IT=1 cargo test -p chvm-lib
    #[tokio::test]
    async fn prepare_creates_overlay_over_shared_base_and_reuses_it() {
        if std::env::var("CHVM_QEMU_IMG_IT").is_err() || super::super::overlay::qemu_img().is_none()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let rootfs = dir.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();
        // Pre-create a real qcow2 base so no mkfs/mount (root) is needed.
        let base = base_image_path(&rootfs);
        let output = std::process::Command::new(super::super::overlay::qemu_img().unwrap())
            .args(["create", "-f", "qcow2"])
            .arg(&base)
            .arg((64 * 1024 * 1024).to_string())
            .output()
            .unwrap();
        assert!(output.status.success());

        let vm_dir = dir.path().join("vms").join("vm1");
        std::fs::create_dir_all(&vm_dir).unwrap();
        let config = PrepareConfig {
            vm_id: "vm1".to_string(),
            rootfs_path: rootfs.to_string_lossy().to_string(),
            state_dir: vm_dir.clone(),
            read_only: false,
            tag: "rootfs".into(),
            inject_mycelium: false,
            inject_runtime_binaries: false,
            block_disk_size_bytes: None,
            writable_quota_bytes: None,
        };

        let result = BlockStorage.prepare(&config).await.unwrap();
        let disk = vm_dir.join(OVERLAY_DISK_NAME);
        assert_eq!(
            result.block_device.as_deref(),
            Some(disk.to_string_lossy().as_ref())
        );
        assert_eq!(
            result.block_format.as_deref(),
            Some(super::super::overlay::QCOW2_FORMAT)
        );
        assert_eq!(
            std::fs::metadata(&base).unwrap().permissions().mode() & 0o777,
            0o444,
            "base must be immutable after prepare"
        );
        let created_mtime = std::fs::metadata(&disk).unwrap().modified().unwrap();

        // Second prepare reuses the overlay instead of recreating it.
        BlockStorage.prepare(&config).await.unwrap();
        assert_eq!(
            std::fs::metadata(&disk).unwrap().modified().unwrap(),
            created_mtime,
            "existing overlay must not be recreated"
        );
        assert_eq!(
            super::super::overlay::backing_file(&disk).unwrap().unwrap(),
            base.canonicalize().unwrap()
        );
    }

    #[test]
    fn cleanup_removes_only_per_vm_disks() {
        let home = tempfile::tempdir().unwrap();
        let vm_dir = home.path().join("vms").join("vm-cleanup");
        std::fs::create_dir_all(&vm_dir).unwrap();
        std::fs::write(vm_dir.join(OVERLAY_DISK_NAME), b"overlay").unwrap();
        std::fs::write(vm_dir.join(FULL_DISK_NAME), b"full").unwrap();
        // A base lookalike outside the VM dir must survive cleanup.
        let lookalike = home.path().join("images").join("rootfs.ext4.img");
        std::fs::create_dir_all(lookalike.parent().unwrap()).unwrap();
        std::fs::write(&lookalike, b"base").unwrap();

        remove_vm_disks(&vm_dir).unwrap();

        assert!(!vm_dir.join(OVERLAY_DISK_NAME).exists());
        assert!(!vm_dir.join(FULL_DISK_NAME).exists());
        assert!(lookalike.exists());
    }
}
