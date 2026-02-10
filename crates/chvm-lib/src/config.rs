use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_base_dir")]
    pub base_dir: PathBuf,

    #[serde(default = "default_vcpu")]
    pub default_vcpu: u32,

    #[serde(default = "default_memory_mb")]
    pub default_memory_mb: u32,

    #[serde(default)]
    pub kernel_path: Option<String>,

    #[serde(default)]
    pub virtiofsd_bin: Option<String>,

    #[serde(default)]
    pub cloud_hypervisor_bin: Option<String>,

    #[serde(default = "default_network_mode")]
    pub default_network_mode: String,

    #[serde(default = "default_storage_backend")]
    pub default_storage_backend: String,

    #[serde(default = "default_bridge")]
    pub default_bridge: String,
}

fn default_base_dir() -> PathBuf {
    Paths::resolve().base
}
fn default_vcpu() -> u32 { 1 }
fn default_memory_mb() -> u32 { 512 }
fn default_network_mode() -> String { "tap".into() }
fn default_storage_backend() -> String { "block".into() }
fn default_bridge() -> String { "chvm0".into() }

impl Default for Config {
    fn default() -> Self {
        Self {
            base_dir: default_base_dir(),
            default_vcpu: default_vcpu(),
            default_memory_mb: default_memory_mb(),
            kernel_path: None,
            virtiofsd_bin: None,
            cloud_hypervisor_bin: None,
            default_network_mode: default_network_mode(),
            default_storage_backend: default_storage_backend(),
            default_bridge: default_bridge(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            let cfg: Config = serde_json::from_str(&data)?;
            Ok(cfg)
        } else {
            Ok(Config::default())
        }
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn paths(&self) -> Paths {
        Paths::new(self.base_dir.clone())
    }
}
