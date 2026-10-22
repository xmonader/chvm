//! qcow2 copy-on-write overlay disks on top of immutable shared base images.
//!
//! A VM's disk is a qcow2 overlay that stores only the blocks differing from
//! a read-only base ext4 image shared by all VMs of the same image. Overlay
//! creation is instantaneous (~200 KB) instead of a full per-VM copy.
//!
//! Rules (see the qcow2 snapshot design doc):
//! - Bases are immutable: `chmod 0444` after build, never modify them while
//!   overlays reference them.
//! - The backing format is passed explicitly (`-F qcow2`); without it qemu
//!   warns and guesses raw, which is a security footgun.
//! - Backing paths are absolute so instance dirs can be moved around.
//! - Resizing is grow-only; shrinking corrupts the guest filesystem.

use std::path::{Path, PathBuf};
use std::process::Command;

use tracing::{info, warn};

/// Disk image format reported for overlays (cloud-hypervisor `image_type=`).
pub const QCOW2_FORMAT: &str = "qcow2";

/// Env var overriding the qemu-img binary location (used by tests).
const QEMU_IMG_ENV: &str = "CHVM_QEMU_IMG";

/// Resolve the qemu-img binary: env override first, then PATH lookup.
pub fn qemu_img() -> Option<PathBuf> {
    if let Ok(override_path) = std::env::var(QEMU_IMG_ENV) {
        let path = PathBuf::from(&override_path);
        if path.is_file() {
            return Some(path);
        }
        warn!(
            path = %override_path,
            "{QEMU_IMG_ENV} is set but does not point to a file, falling back to PATH lookup"
        );
    }
    which::which("qemu-img").ok()
}

/// Argument vector for `qemu-img create` (pure, for testability).
fn create_args(base: &Path, dest: &Path) -> Vec<String> {
    vec![
        "create".to_string(),
        "-f".to_string(),
        QCOW2_FORMAT.to_string(),
        "-F".to_string(),
        QCOW2_FORMAT.to_string(),
        "-b".to_string(),
        base.to_string_lossy().into_owned(),
        dest.to_string_lossy().into_owned(),
    ]
}

/// Argument vector for `qemu-img resize` (pure, for testability).
fn resize_args(disk: &Path, size_bytes: u64) -> Vec<String> {
    vec![
        "resize".to_string(),
        disk.to_string_lossy().into_owned(),
        size_bytes.to_string(),
    ]
}

/// Argument vector for `qemu-img info --output=json` (pure, for testability).
fn info_args(disk: &Path) -> Vec<String> {
    vec![
        "info".to_string(),
        "--output=json".to_string(),
        disk.to_string_lossy().into_owned(),
    ]
}

/// Subset of the `qemu-img info --output=json` payload we consume.
#[derive(Debug, serde::Deserialize)]
struct QemuImgInfo {
    #[serde(rename = "virtual-size")]
    virtual_size: Option<u64>,
    #[serde(rename = "backing-filename")]
    backing_filename: Option<String>,
}

fn parse_info(json: &str) -> anyhow::Result<QemuImgInfo> {
    Ok(serde_json::from_str(json)?)
}

fn run(qemu_img: &Path, args: &[String], what: &str) -> anyhow::Result<()> {
    let output = Command::new(qemu_img).args(args).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "qemu-img {what} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn info(qemu_img: &Path, disk: &Path) -> anyhow::Result<QemuImgInfo> {
    let output = Command::new(qemu_img).args(info_args(disk)).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "qemu-img info failed for {}: {}",
            disk.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    parse_info(&String::from_utf8_lossy(&output.stdout))
}

/// Create a qcow2 overlay at `dest` with `base` as its backing file.
///
/// The base path is canonicalized so the overlay records an absolute backing
/// path that survives moves of the VM dir. When `size_bytes` is given and
/// exceeds the overlay's virtual size, it is grown (grow-only).
pub fn create_overlay(base: &Path, dest: &Path, size_bytes: Option<u64>) -> anyhow::Result<()> {
    let qemu_img = qemu_img().ok_or_else(|| anyhow::anyhow!("qemu-img not found"))?;
    let base_abs = std::fs::canonicalize(base)
        .map_err(|e| anyhow::anyhow!("canonicalize base image {}: {e}", base.display()))?;

    run(&qemu_img, &create_args(&base_abs, dest), "create")?;
    info!(
        base = %base_abs.display(),
        dest = %dest.display(),
        "Created qcow2 overlay disk"
    );

    if let Some(size) = size_bytes {
        resize_grow_only_with(&qemu_img, dest, size)?;
    }
    Ok(())
}

/// Grow `disk` to `size_bytes` if that is larger than its current virtual
/// size. A smaller request is logged and skipped (shrinking would corrupt
/// the guest filesystem).
pub fn resize_grow_only(disk: &Path, size_bytes: u64) -> anyhow::Result<()> {
    let qemu_img = qemu_img().ok_or_else(|| anyhow::anyhow!("qemu-img not found"))?;
    resize_grow_only_with(&qemu_img, disk, size_bytes)
}

fn resize_grow_only_with(qemu_img: &Path, disk: &Path, size_bytes: u64) -> anyhow::Result<()> {
    let current = info(qemu_img, disk)?.virtual_size.unwrap_or(0);
    if size_bytes <= current {
        if size_bytes < current {
            warn!(
                disk = %disk.display(),
                requested = size_bytes,
                current = current,
                "Requested disk size is smaller than the current virtual size; skipping resize"
            );
        }
        return Ok(());
    }

    run(qemu_img, &resize_args(disk, size_bytes), "resize")?;
    info!(
        disk = %disk.display(),
        size = size_bytes,
        "Resized overlay disk (grow-only)"
    );
    Ok(())
}

/// Return the backing file recorded in `disk`, if any.
pub fn backing_file(disk: &Path) -> anyhow::Result<Option<PathBuf>> {
    let qemu_img = qemu_img().ok_or_else(|| anyhow::anyhow!("qemu-img not found"))?;
    Ok(info(&qemu_img, disk)?.backing_filename.map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn create_args_passes_explicit_format_and_backing_file() {
        let args = create_args(
            Path::new("/data/images/digest/rootfs.ext4.img"),
            Path::new("/data/vms/vm1/rootfs.qcow2"),
        );
        assert_eq!(
            args,
            vec![
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                "/data/images/digest/rootfs.ext4.img",
                "/data/vms/vm1/rootfs.qcow2",
            ]
        );
    }

    #[test]
    fn resize_args_use_plain_bytes() {
        let args = resize_args(Path::new("/data/vms/vm1/rootfs.qcow2"), 128 * MIB);
        assert_eq!(
            args,
            vec![
                "resize",
                "/data/vms/vm1/rootfs.qcow2",
                &(128 * MIB).to_string(),
            ]
        );
    }

    #[test]
    fn info_args_request_json_output() {
        let args = info_args(Path::new("/data/vms/vm1/rootfs.qcow2"));
        assert_eq!(
            args,
            vec!["info", "--output=json", "/data/vms/vm1/rootfs.qcow2"]
        );
    }

    #[test]
    fn parse_info_reads_backing_file_and_virtual_size() {
        let json = r#"{
            "virtual-size": 134217728,
            "filename": "/data/vms/vm1/rootfs.qcow2",
            "format": "qcow2",
            "backing-filename": "/data/images/digest/rootfs.ext4.img"
        }"#;
        let info = parse_info(json).unwrap();
        assert_eq!(info.virtual_size, Some(128 * MIB));
        assert_eq!(
            info.backing_filename.as_deref(),
            Some("/data/images/digest/rootfs.ext4.img")
        );
    }

    #[test]
    fn parse_info_without_backing_file() {
        let json = r#"{"virtual-size": 67108864, "format": "qcow2"}"#;
        let info = parse_info(json).unwrap();
        assert_eq!(info.virtual_size, Some(64 * MIB));
        assert_eq!(info.backing_filename, None);
    }

    #[test]
    fn parse_info_rejects_malformed_json() {
        assert!(parse_info("not json").is_err());
    }

    fn qemu_img_tests_enabled() -> Option<PathBuf> {
        if std::env::var("CHVM_QEMU_IMG_IT").is_err() {
            return None;
        }
        qemu_img()
    }

    fn create_base(dir: &Path, name: &str, size: u64) -> PathBuf {
        let base = dir.join(name);
        std::process::Command::new(qemu_img().unwrap())
            .args(["create", "-f", "qcow2"])
            .arg(&base)
            .arg(size.to_string())
            .output()
            .expect("run qemu-img create for base");
        base
    }

    fn virtual_size(disk: &Path) -> u64 {
        let output = std::process::Command::new(qemu_img().unwrap())
            .args(["info", "--output=json"])
            .arg(disk)
            .output()
            .expect("run qemu-img info");
        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse qemu-img info");
        json["virtual-size"].as_u64().expect("virtual-size")
    }

    /// Env-gated integration test against the real qemu-img binary:
    /// CHVM_QEMU_IMG_IT=1 cargo test -p chvm-lib
    #[test]
    fn creates_overlay_with_absolute_backing_file_and_grows() {
        let Some(_qemu) = qemu_img_tests_enabled() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let base = create_base(dir.path(), "base.qcow2", 64 * MIB);

        // A non-canonical base path must still be recorded absolute.
        let dotty_base = base.parent().unwrap().join(".").join("base.qcow2");
        let overlay = dir.path().join("vm").join("rootfs.qcow2");
        std::fs::create_dir_all(overlay.parent().unwrap()).unwrap();
        create_overlay(&dotty_base, &overlay, Some(128 * MIB)).unwrap();

        assert!(overlay.exists());
        assert!(
            overlay.metadata().unwrap().len() < MIB,
            "overlay should be small, got {} bytes",
            overlay.metadata().unwrap().len()
        );
        let backing = backing_file(&overlay).unwrap().unwrap();
        assert_eq!(backing, base.canonicalize().unwrap());
        assert!(backing.is_absolute());
        assert_eq!(virtual_size(&overlay), 128 * MIB);
    }

    /// Env-gated integration test: a resize smaller than the current virtual
    /// size is skipped, never applied.
    #[test]
    fn resize_never_shrinks() {
        let Some(_qemu) = qemu_img_tests_enabled() else {
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let base = create_base(dir.path(), "base.qcow2", 64 * MIB);
        let overlay = dir.path().join("rootfs.qcow2");
        create_overlay(&base, &overlay, Some(32 * MIB)).unwrap();

        assert_eq!(virtual_size(&overlay), 64 * MIB);
    }
}
