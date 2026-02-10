use std::path::PathBuf;

use crate::vm::state::PortForward;

/// Configuration for setting up networking.
pub struct SetupConfig {
    pub vm_id: String,
    pub mode: String,
    pub state_dir: PathBuf,
    pub mac_address: Option<String>,
    pub port_forwards: Vec<PortForward>,
    pub mycelium_peers: Vec<String>,
    pub bridge: Option<String>,
}

/// Result of network setup.
#[derive(Debug, Clone)]
pub struct NetworkResult {
    pub tap_device: Option<String>,
    pub mac_address: Option<String>,
    pub ip_address: Option<String>,
    pub gateway: Option<String>,
    pub bridge: Option<String>,
    pub mycelium_enabled: bool,
    pub mycelium_peers: Vec<String>,
}

/// Configuration for tearing down networking.
pub struct TeardownConfig {
    pub vm_id: String,
    pub ip_address: Option<String>,
    pub port_forwards: Vec<PortForward>,
}

/// Network backend trait.
#[async_trait::async_trait]
pub trait NetworkBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn setup(&self, config: &SetupConfig) -> anyhow::Result<NetworkResult>;
    async fn teardown(&self, config: &TeardownConfig) -> anyhow::Result<()>;
}
