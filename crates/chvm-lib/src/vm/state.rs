use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmState {
    Created,
    Running,
    Stopped,
    Failed,
}

impl std::fmt::Display for VmState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmState::Created => write!(f, "created"),
            VmState::Running => write!(f, "running"),
            VmState::Stopped => write!(f, "stopped"),
            VmState::Failed => write!(f, "failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RootfsType {
    #[serde(rename = "virtioblock")]
    VirtioBlock,
    #[serde(rename = "virtiofs")]
    VirtioFs,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    #[default]
    Tap,
    Mycelium,
    None,
}

impl std::fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetworkMode::Tap => write!(f, "tap"),
            NetworkMode::Mycelium => write!(f, "mycelium"),
            NetworkMode::None => write!(f, "none"),
        }
    }
}

impl std::str::FromStr for NetworkMode {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "tap" => Ok(NetworkMode::Tap),
            "mycelium" => Ok(NetworkMode::Mycelium),
            "none" => Ok(NetworkMode::None),
            _ => Err(format!("unknown network mode: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KernelConfig {
    pub path: String,
    pub cmdline: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initrd_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootfsConfig {
    pub source: String,
    #[serde(rename = "type")]
    pub rootfs_type: RootfsType,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForward {
    pub host_port: u16,
    pub guest_port: u16,
    #[serde(default = "default_protocol")]
    pub protocol: String,
}

fn default_protocol() -> String {
    "tcp".into()
}

impl std::str::FromStr for PortForward {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse formats: "8080:80", "8080:80/tcp", "8080:80/udp"
        let (ports, proto) = if let Some(idx) = s.rfind('/') {
            (&s[..idx], &s[idx + 1..])
        } else {
            (s, "tcp")
        };

        let parts: Vec<&str> = ports.split(':').collect();
        if parts.len() != 2 {
            return Err(format!(
                "invalid port forward format: {s}, expected HOST:GUEST"
            ));
        }

        let host_port: u16 = parts[0]
            .parse()
            .map_err(|_| format!("invalid host port: {}", parts[0]))?;
        let guest_port: u16 = parts[1]
            .parse()
            .map_err(|_| format!("invalid guest port: {}", parts[1]))?;

        Ok(PortForward {
            host_port,
            guest_port,
            protocol: proto.to_string(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub mode: NetworkMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub port_forwards: Vec<PortForward>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mycelium_peers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mycelium_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InitConfig {
    pub cmd: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub source: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub mount_type: Option<String>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl std::str::FromStr for MountConfig {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse format: "/host/path:/guest/path" or "/host/path:/guest/path:ro"
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() < 2 {
            return Err(format!(
                "invalid volume format: {s}, expected SOURCE:TARGET[:ro]"
            ));
        }

        let read_only = parts.len() > 2 && parts[2] == "ro";

        Ok(MountConfig {
            source: parts[0].to_string(),
            target: parts[1].to_string(),
            mount_type: None,
            read_only,
            tag: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub id: String,
    pub name: String,
    pub vcpu: u32,
    pub memory_mb: u32,
    pub kernel: KernelConfig,
    pub rootfs: RootfsConfig,
    pub network: NetworkConfig,
    pub init: InitConfig,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mounts: Vec<MountConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_path: Option<String>,
    #[serde(default)]
    pub vsock_cid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // PortForward parsing tests

    #[test]
    fn port_forward_basic() {
        let pf: PortForward = "8080:80".parse().unwrap();
        assert_eq!(pf.host_port, 8080);
        assert_eq!(pf.guest_port, 80);
        assert_eq!(pf.protocol, "tcp");
    }

    #[test]
    fn port_forward_with_tcp() {
        let pf: PortForward = "3000:3000/tcp".parse().unwrap();
        assert_eq!(pf.host_port, 3000);
        assert_eq!(pf.guest_port, 3000);
        assert_eq!(pf.protocol, "tcp");
    }

    #[test]
    fn port_forward_with_udp() {
        let pf: PortForward = "5353:53/udp".parse().unwrap();
        assert_eq!(pf.host_port, 5353);
        assert_eq!(pf.guest_port, 53);
        assert_eq!(pf.protocol, "udp");
    }

    #[test]
    fn port_forward_missing_colon() {
        let result: Result<PortForward, _> = "8080".parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("HOST:GUEST"));
    }

    #[test]
    fn port_forward_invalid_host_port() {
        let result: Result<PortForward, _> = "abc:80".parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid host port"));
    }

    #[test]
    fn port_forward_invalid_guest_port() {
        let result: Result<PortForward, _> = "8080:xyz".parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid guest port"));
    }

    #[test]
    fn port_forward_port_out_of_range() {
        let result: Result<PortForward, _> = "99999:80".parse();
        assert!(result.is_err());
    }

    // MountConfig parsing tests

    #[test]
    fn mount_config_basic() {
        let mc: MountConfig = "/host/data:/guest/data".parse().unwrap();
        assert_eq!(mc.source, "/host/data");
        assert_eq!(mc.target, "/guest/data");
        assert!(!mc.read_only);
        assert!(mc.mount_type.is_none());
        assert!(mc.tag.is_none());
    }

    #[test]
    fn mount_config_read_only() {
        let mc: MountConfig = "/src:/dest:ro".parse().unwrap();
        assert_eq!(mc.source, "/src");
        assert_eq!(mc.target, "/dest");
        assert!(mc.read_only);
    }

    #[test]
    fn mount_config_rw_explicit() {
        let mc: MountConfig = "/src:/dest:rw".parse().unwrap();
        assert_eq!(mc.source, "/src");
        assert_eq!(mc.target, "/dest");
        assert!(!mc.read_only);
    }

    #[test]
    fn mount_config_missing_target() {
        let result: Result<MountConfig, _> = "/only-source".parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("SOURCE:TARGET"));
    }

    // VmState display tests

    #[test]
    fn vm_state_display() {
        assert_eq!(VmState::Created.to_string(), "created");
        assert_eq!(VmState::Running.to_string(), "running");
        assert_eq!(VmState::Stopped.to_string(), "stopped");
        assert_eq!(VmState::Failed.to_string(), "failed");
    }

    // NetworkMode tests

    #[test]
    fn network_mode_from_str() {
        assert_eq!("tap".parse::<NetworkMode>().unwrap(), NetworkMode::Tap);
        assert_eq!(
            "mycelium".parse::<NetworkMode>().unwrap(),
            NetworkMode::Mycelium
        );
        assert_eq!("none".parse::<NetworkMode>().unwrap(), NetworkMode::None);
    }

    #[test]
    fn network_mode_from_str_invalid() {
        let result: Result<NetworkMode, _> = "invalid".parse();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown network mode"));
    }

    #[test]
    fn network_mode_display() {
        assert_eq!(NetworkMode::Tap.to_string(), "tap");
        assert_eq!(NetworkMode::Mycelium.to_string(), "mycelium");
        assert_eq!(NetworkMode::None.to_string(), "none");
    }

    #[test]
    fn network_mode_default() {
        let mode = NetworkMode::default();
        assert_eq!(mode, NetworkMode::Tap);
    }

    // VmConfig tests

    #[test]
    fn vm_config_api_socket_path() {
        let config = VmConfig {
            id: "test-vm".to_string(),
            name: "test".to_string(),
            vcpu: 1,
            memory_mb: 512,
            kernel: KernelConfig {
                path: "/kernel".to_string(),
                cmdline: "console=ttyS0".to_string(),
                initrd_path: None,
            },
            rootfs: RootfsConfig {
                source: "/rootfs".to_string(),
                rootfs_type: RootfsType::VirtioBlock,
                read_only: false,
            },
            network: NetworkConfig {
                mode: NetworkMode::Tap,
                tap_device: None,
                mac_address: None,
                ip_address: None,
                gateway: None,
                port_forwards: vec![],
                mycelium_peers: vec![],
                mycelium_key: None,
            },
            init: InitConfig {
                cmd: vec!["/bin/sh".to_string()],
                env: vec![],
                working_dir: None,
            },
            mounts: vec![],
            log_path: None,
            vsock_cid: 3,
            state_dir: Some("/state/test-vm".to_string()),
        };

        assert_eq!(
            config.api_socket_path(),
            Some("/state/test-vm/api.sock".to_string())
        );
    }

    #[test]
    fn vm_config_vsock_socket_path() {
        let config = VmConfig {
            id: "test-vm".to_string(),
            name: "test".to_string(),
            vcpu: 1,
            memory_mb: 512,
            kernel: KernelConfig {
                path: "/kernel".to_string(),
                cmdline: "console=ttyS0".to_string(),
                initrd_path: None,
            },
            rootfs: RootfsConfig {
                source: "/rootfs".to_string(),
                rootfs_type: RootfsType::VirtioBlock,
                read_only: false,
            },
            network: NetworkConfig {
                mode: NetworkMode::Tap,
                tap_device: None,
                mac_address: None,
                ip_address: None,
                gateway: None,
                port_forwards: vec![],
                mycelium_peers: vec![],
                mycelium_key: None,
            },
            init: InitConfig {
                cmd: vec!["/bin/sh".to_string()],
                env: vec![],
                working_dir: None,
            },
            mounts: vec![],
            log_path: None,
            vsock_cid: 3,
            state_dir: Some("/state/test-vm".to_string()),
        };

        assert_eq!(
            config.vsock_socket_path(),
            Some("/state/test-vm/vsock.sock".to_string())
        );
    }

    #[test]
    fn vm_config_no_state_dir() {
        let config = VmConfig {
            id: "test-vm".to_string(),
            name: "test".to_string(),
            vcpu: 1,
            memory_mb: 512,
            kernel: KernelConfig {
                path: "/kernel".to_string(),
                cmdline: "console=ttyS0".to_string(),
                initrd_path: None,
            },
            rootfs: RootfsConfig {
                source: "/rootfs".to_string(),
                rootfs_type: RootfsType::VirtioBlock,
                read_only: false,
            },
            network: NetworkConfig {
                mode: NetworkMode::Tap,
                tap_device: None,
                mac_address: None,
                ip_address: None,
                gateway: None,
                port_forwards: vec![],
                mycelium_peers: vec![],
                mycelium_key: None,
            },
            init: InitConfig {
                cmd: vec!["/bin/sh".to_string()],
                env: vec![],
                working_dir: None,
            },
            mounts: vec![],
            log_path: None,
            vsock_cid: 3,
            state_dir: None,
        };

        assert_eq!(config.api_socket_path(), None);
        assert_eq!(config.vsock_socket_path(), None);
    }

    // Serialization tests

    #[test]
    fn vm_state_serialization() {
        let state = VmState::Running;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"running\"");

        let deserialized: VmState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, VmState::Running);
    }

    #[test]
    fn network_mode_serialization() {
        let mode = NetworkMode::Mycelium;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"mycelium\"");

        let deserialized: NetworkMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, NetworkMode::Mycelium);
    }

    #[test]
    fn rootfs_type_serialization() {
        let rt = RootfsType::VirtioFs;
        let json = serde_json::to_string(&rt).unwrap();
        assert_eq!(json, "\"virtiofs\"");

        let deserialized: RootfsType = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, RootfsType::VirtioFs);
    }

    #[test]
    fn port_forward_serialization() {
        let pf = PortForward {
            host_port: 8080,
            guest_port: 80,
            protocol: "tcp".to_string(),
        };

        let json = serde_json::to_string(&pf).unwrap();
        let deserialized: PortForward = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.host_port, 8080);
        assert_eq!(deserialized.guest_port, 80);
        assert_eq!(deserialized.protocol, "tcp");
    }

    #[test]
    fn default_protocol_is_tcp() {
        let pf: PortForward = "8080:80".parse().unwrap();
        assert_eq!(pf.protocol, "tcp");
    }
}

impl VmConfig {
    /// Path to the Cloud Hypervisor API socket.
    pub fn api_socket_path(&self) -> Option<String> {
        self.state_dir.as_deref().map(|d| format!("{d}/api.sock"))
    }

    /// Path to the vsock UDS proxy socket.
    pub fn vsock_socket_path(&self) -> Option<String> {
        self.state_dir.as_deref().map(|d| format!("{d}/vsock.sock"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway: Option<String>,
    #[serde(default)]
    pub mycelium_enabled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mycelium_peers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mycelium_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtiofs_socket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub virtiofs_pid: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeState {
    pub id: String,
    pub name: String,
    pub image_ref: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_digest: Option<String>,
    pub vm_config: VmConfig,
    pub status: VmState,
    #[serde(default)]
    pub vmm_pid: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_state: Option<NetworkState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_state: Option<StorageState>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<DateTime<Utc>>,
}
