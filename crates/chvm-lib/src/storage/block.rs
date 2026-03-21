use std::path::Path;

use crate::storage::traits::*;

/// Virtio-block storage backend.
pub struct BlockStorage;

#[async_trait::async_trait]
impl StorageBackend for BlockStorage {
    fn name(&self) -> &str {
        "block"
    }

    async fn prepare(&self, config: &PrepareConfig) -> anyhow::Result<StorageResult> {
        let block_path = config.state_dir.join("rootfs.img");

        if !block_path.exists() {
            super::image::create_ext4_image(
                &block_path,
                Path::new(&config.rootfs_path),
                config.inject_mycelium,
                config.block_disk_size_bytes,
            )?;
        }

        Ok(StorageResult {
            block_device: Some(block_path.to_string_lossy().to_string()),
            socket_path: None,
            virtiofs_pid: None,
            overlay_dir: None,
            overlay_backing_dir: None,
        })
    }

    async fn cleanup(&self, _vm_id: &str) -> anyhow::Result<()> {
        Ok(())
    }
}
