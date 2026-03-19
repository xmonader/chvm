use std::path::Path;
use std::process::{Command, Stdio};

use tracing::info;

use crate::storage::image::inject_runtime_binaries;
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
        let upper_dir = overlay_dir.join("upper");
        let work_dir = overlay_dir.join("work");
        let merged_dir = overlay_dir.join("merged");

        std::fs::create_dir_all(&upper_dir)?;
        std::fs::create_dir_all(&work_dir)?;
        std::fs::create_dir_all(&merged_dir)?;

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
            anyhow::bail!(
                "overlay mount failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }

        if config.inject_runtime_binaries {
            inject_runtime_binaries(Path::new(&merged_dir), config.inject_mycelium);
        }

        // Start virtiofsd
        let socket_path = storage_dir.join("virtiofsd.sock");
        let virtiofsd_bin = match find_virtiofsd() {
            Ok(bin) => bin,
            Err(e) => {
                // Unmount overlay before returning error
                let _ = Command::new("umount").arg(&merged_dir).output();
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
                    socket_path: Some(socket_path.to_string_lossy().to_string()),
                    virtiofs_pid: Some(pid),
                    overlay_dir: Some(overlay_dir.to_string_lossy().to_string()),
                });
            }
            if let Some(status) = child.try_wait()? {
                let _ = Command::new("umount").arg(&merged_dir).output();
                anyhow::bail!("virtiofsd exited before socket became ready: {status}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        let _ = child.kill();
        let _ = child.wait();
        let _ = Command::new("umount").arg(&merged_dir).output();
        anyhow::bail!(
            "virtiofsd socket did not become ready: {}",
            socket_path.display()
        )
    }

    async fn cleanup(&self, _vm_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

fn find_virtiofsd() -> anyhow::Result<String> {
    crate::utils::find_command(
        "virtiofsd",
        &[
            "/usr/libexec/virtiofsd",
            "/usr/lib/virtiofsd",
            "/usr/local/bin/virtiofsd",
            "/usr/bin/virtiofsd",
        ],
    )
    .ok_or_else(|| anyhow::anyhow!("virtiofsd not found. Install virtiofsd."))
}
