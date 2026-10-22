use std::path::PathBuf;

/// Configuration for preparing storage.
pub struct PrepareConfig {
    pub vm_id: String,
    pub rootfs_path: String,
    pub state_dir: PathBuf,
    pub read_only: bool,
    pub tag: String,
    pub inject_mycelium: bool,
    pub inject_runtime_binaries: bool,
    pub block_disk_size_bytes: Option<u64>,
    pub writable_quota_bytes: Option<u64>,
}

/// Result of storage preparation.
pub struct StorageResult {
    pub block_device: Option<String>,
    pub socket_path: Option<String>,
    pub virtiofs_pid: Option<i32>,
    pub overlay_dir: Option<String>,
    pub overlay_backing_dir: Option<String>,
}

/// Storage backend trait.
// async_trait stamps `#[must_use]` on methods that already return a must-use
// boxed future; newer clippy's double_must_use flags the expansion.
#[allow(clippy::double_must_use)]
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn prepare(&self, config: &PrepareConfig) -> anyhow::Result<StorageResult>;
    async fn cleanup(&self, vm_id: &str) -> anyhow::Result<()>;
}
