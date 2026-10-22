use std::path::Path;
use std::process::{Command, Stdio};

use tracing::info;

use crate::storage::image::{create_empty_ext4_image, inject_binary, inject_guest_bootstrap};
use crate::storage::traits::*;

/// Virtio-FS storage backend using virtiofsd.
pub struct VirtioFsStorage;

#[async_trait::async_trait]
impl StorageBackend for VirtioFsStorage {
    fn name(&self) -> &str {
        "virtiofs"
    }

    async fn prepare(&self, config: &PrepareConfig) -> anyhow::Result<StorageResult> {
        let storage_dir = config.state_dir.join("storage");
        std::fs::create_dir_all(&storage_dir)?;

        // Set up overlay filesystem
        let overlay_dir = storage_dir.join("overlay");
        let quota_backing_dir = overlay_dir.join("quota-backing");
        let quota_backing_img = overlay_dir.join("quota-backing.img");
        let merged_dir = overlay_dir.join("merged");
        std::fs::create_dir_all(&merged_dir)?;

        let (upper_dir, work_dir, overlay_backing_dir) =
            if let Some(quota_bytes) = config.writable_quota_bytes {
                std::fs::create_dir_all(&quota_backing_dir)?;
                create_empty_ext4_image(&quota_backing_img, quota_bytes, "chvm-upper")?;

                let output = Command::new("mount")
                    .args(["-o", "loop"])
                    .arg(&quota_backing_img)
                    .arg(&quota_backing_dir)
                    .output()?;
                if !output.status.success() {
                    anyhow::bail!(
                        "quota backing mount failed: {}",
                        String::from_utf8_lossy(&output.stderr)
                    );
                }

                let upper_dir = quota_backing_dir.join("upper");
                let work_dir = quota_backing_dir.join("work");
                std::fs::create_dir_all(&upper_dir)?;
                std::fs::create_dir_all(&work_dir)?;
                (
                    upper_dir,
                    work_dir,
                    Some(quota_backing_dir.to_string_lossy().to_string()),
                )
            } else {
                let upper_dir = overlay_dir.join("upper");
                let work_dir = overlay_dir.join("work");
                std::fs::create_dir_all(&upper_dir)?;
                std::fs::create_dir_all(&work_dir)?;
                (upper_dir, work_dir, None)
            };

        // Mount overlay
        let overlay_opts = format!(
            "lowerdir={},upperdir={},workdir={}",
            config.rootfs_path,
            upper_dir.display(),
            work_dir.display()
        );

        let output = Command::new("mount")
            .args(["-t", "overlay", "overlay", "-o", &overlay_opts])
            .arg(&merged_dir)
            .output()?;

        if !output.status.success() {
            if overlay_backing_dir.is_some() {
                let _ = Command::new("umount").arg(&quota_backing_dir).output();
            }
            anyhow::bail!(
                "overlay mount failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        if config.inject_runtime_binaries {
            inject_guest_bootstrap(Path::new(&merged_dir), config.inject_mycelium);
        } else if config.inject_mycelium {
            inject_binary(
                Path::new(&merged_dir),
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

        // Start virtiofsd
        let socket_path = storage_dir.join("virtiofsd.sock");
        let virtiofsd_bin = match find_virtiofsd_path(None) {
            Ok(bin) => bin,
            Err(e) => {
                // Unmount overlay before returning error
                let _ = Command::new("umount").arg(&merged_dir).output();
                if overlay_backing_dir.is_some() {
                    let _ = Command::new("umount").arg(&quota_backing_dir).output();
                }
                return Err(e);
            }
        };

        let mut child = match Command::new(&virtiofsd_bin)
            .args([
                &format!("--socket-path={}", socket_path.display()),
                &format!("--shared-dir={}", merged_dir.display()),
                "--cache=auto",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => child,
            Err(e) => {
                // Unmount overlay before returning error
                let _ = Command::new("umount").arg(&merged_dir).output();
                if overlay_backing_dir.is_some() {
                    let _ = Command::new("umount").arg(&quota_backing_dir).output();
                }
                return Err(e.into());
            }
        };

        let pid = child.id() as i32;

        // Wait for socket (use tokio::time::sleep to avoid blocking the async runtime)
        for _ in 0..50 {
            if socket_path.exists() {
                info!(pid = pid, socket = %socket_path.display(), "virtiofsd started");
                return Ok(StorageResult {
                    block_device: None,
                    block_format: None,
                    socket_path: Some(socket_path.to_string_lossy().to_string()),
                    virtiofs_pid: Some(pid),
                    overlay_dir: Some(overlay_dir.to_string_lossy().to_string()),
                    overlay_backing_dir,
                });
            }
            if let Some(status) = child.try_wait()? {
                let _ = Command::new("umount").arg(&merged_dir).output();
                if overlay_backing_dir.is_some() {
                    let _ = Command::new("umount").arg(&quota_backing_dir).output();
                }
                anyhow::bail!("virtiofsd exited before socket became ready: {status}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let _ = child.kill();
        let _ = child.wait();
        let _ = Command::new("umount").arg(&merged_dir).output();
        if overlay_backing_dir.is_some() {
            let _ = Command::new("umount").arg(&quota_backing_dir).output();
        }
        anyhow::bail!(
            "virtiofsd socket did not become ready: {}",
            socket_path.display()
        )
    }

    async fn cleanup(&self, _vm_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

pub fn find_virtiofsd_path(configured: Option<&str>) -> anyhow::Result<String> {
    if let Some(path) = configured {
        if Path::new(path).exists() {
            return Ok(path.to_string());
        }
    }

    crate::utils::find_command(
        "virtiofsd",
        &[
            "/usr/libexec/virtiofsd",
            "/usr/lib/virtiofsd",
            "/usr/lib/qemu/virtiofsd",
            "/usr/local/bin/virtiofsd",
            "/usr/bin/virtiofsd",
        ],
    )
    .ok_or_else(|| anyhow::anyhow!("virtiofsd not found. Install virtiofsd."))
}
