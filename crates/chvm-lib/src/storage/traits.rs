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
}

/// Result of storage preparation.
pub struct StorageResult {
    pub block_device: Option<String>,
    pub socket_path: Option<String>,
    pub virtiofs_pid: Option<i32>,
    pub overlay_dir: Option<String>,
}

/// Storage backend trait.
#[async_trait::async_trait]
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn prepare(&self, config: &PrepareConfig) -> anyhow::Result<StorageResult>;
    async fn cleanup(&self, vm_id: &str) -> anyhow::Result<()>;
}
