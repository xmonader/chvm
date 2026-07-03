use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::paths::Paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

    #[serde(default)]
    pub default_block_disk_size_mb: Option<u64>,

    #[serde(default)]
    pub default_virtiofs_quota_mb: Option<u64>,

    #[serde(default = "default_bridge")]
    pub default_bridge: String,

    #[serde(default = "default_mycelium_peers")]
    pub default_mycelium_peers: Vec<String>,
}

fn default_base_dir() -> PathBuf {
    Paths::resolve().base
}
fn default_vcpu() -> u32 {
    1
}
fn default_memory_mb() -> u32 {
    512
}
fn default_network_mode() -> String {
    "mycelium".into()
}
fn default_storage_backend() -> String {
    "virtiofs".into()
}
fn default_bridge() -> String {
    "chvm0".into()
}
fn default_mycelium_peers() -> Vec<String> {
    vec![
        "tcp://188.40.132.242:9651".into(),
        "tcp://136.243.47.186:9651".into(),
    ]
}

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
            default_block_disk_size_mb: None,
            default_virtiofs_quota_mb: None,
            default_bridge: default_bridge(),
            default_mycelium_peers: default_mycelium_peers(),
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        if path.exists() {
            let data = std::fs::read_to_string(path)?;
            let cfg: Config = serde_json::from_str(&data).map_err(|e| {
                anyhow::anyhow!(
                    "config at {} is invalid ({e}); fix or delete it",
                    path.display()
                )
            })?;
            cfg.validate()?;
            Ok(cfg)
        } else {
            Ok(Config::default())
        }
    }

    /// Reject nonsensical values that would otherwise only fail later at VM boot.
    fn validate(&self) -> anyhow::Result<()> {
        if self.default_vcpu == 0 {
            anyhow::bail!("default_vcpu must be >= 1");
        }
        if self.default_memory_mb == 0 {
            anyhow::bail!("default_memory_mb must be >= 1");
        }
        match self.default_network_mode.as_str() {
            "mycelium" | "tap" | "none" => {}
            other => anyhow::bail!(
                "invalid default_network_mode '{other}' (expected mycelium, tap, or none)"
            ),
        }
        Ok(())
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let data = serde_json::to_string_pretty(self)?;
        crate::utils::atomic_write(path, data.as_bytes())?;
        crate::paths::chown_to_invoker(path);
        Ok(())
    }

    pub fn paths(&self) -> Paths {
        Paths::new(self.base_dir.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.default_vcpu, 1);
        assert_eq!(config.default_memory_mb, 512);
        assert_eq!(config.default_network_mode, "mycelium");
        assert_eq!(config.default_storage_backend, "virtiofs");
        assert_eq!(config.default_block_disk_size_mb, None);
        assert_eq!(config.default_virtiofs_quota_mb, None);
        assert_eq!(config.default_bridge, "chvm0");
        assert!(config.kernel_path.is_none());
        assert!(config.virtiofsd_bin.is_none());
        assert!(config.cloud_hypervisor_bin.is_none());
        assert!(!config.default_mycelium_peers.is_empty());
    }

    #[test]
    fn test_load_nonexistent_returns_default() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("nonexistent.json");

        let config = Config::load(&config_path).unwrap();
        assert_eq!(config.default_vcpu, 1);
        assert_eq!(config.default_memory_mb, 512);
    }

    #[test]
    fn test_save_and_load() {
        let temp_dir = TempDir::new().unwrap();
        let config_path = temp_dir.path().join("config.json");

        let config = Config {
            default_vcpu: 4,
            default_memory_mb: 2048,
            kernel_path: Some("/custom/kernel".to_string()),
            default_bridge: "mybr0".to_string(),
            ..Config::default()
        };

        config.save(&config_path).unwrap();
        assert!(config_path.exists());

        let loaded = Config::load(&config_path).unwrap();
        assert_eq!(loaded.default_vcpu, 4);
        assert_eq!(loaded.default_memory_mb, 2048);
        assert_eq!(loaded.kernel_path, Some("/custom/kernel".to_string()));
        assert_eq!(loaded.default_bridge, "mybr0");
    }

    #[test]
    fn test_config_serialization() {
        let config = Config {
            base_dir: PathBuf::from("/test/base"),
            default_vcpu: 2,
            default_memory_mb: 1024,
            kernel_path: Some("/test/kernel".to_string()),
            virtiofsd_bin: Some("/usr/bin/virtiofsd".to_string()),
            cloud_hypervisor_bin: None,
            default_network_mode: "mycelium".to_string(),
            default_storage_backend: "virtiofs".to_string(),
            default_block_disk_size_mb: Some(2048),
            default_virtiofs_quota_mb: Some(4096),
            default_bridge: "testbr0".to_string(),
            default_mycelium_peers: default_mycelium_peers(),
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: Config = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.default_vcpu, 2);
        assert_eq!(deserialized.default_memory_mb, 1024);
        assert_eq!(deserialized.default_network_mode, "mycelium");
    }

    #[test]
    fn test_paths_from_config() {
        let config = Config {
            base_dir: PathBuf::from("/custom/chvm"),
            ..Config::default()
        };

        let paths = config.paths();
        assert_eq!(paths.base, PathBuf::from("/custom/chvm"));
        assert_eq!(paths.images, PathBuf::from("/custom/chvm/images"));
        assert_eq!(paths.vms, PathBuf::from("/custom/chvm/vms"));
        assert_eq!(paths.kernels, PathBuf::from("/custom/chvm/kernels"));
    }
}
