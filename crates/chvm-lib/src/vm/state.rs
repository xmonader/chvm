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
    Tap,
    #[default]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MyceliumHealth {
    Healthy,
    Unhealthy,
    Unknown,
}

impl std::fmt::Display for MyceliumHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MyceliumHealth::Healthy => write!(f, "healthy"),
            MyceliumHealth::Unhealthy => write!(f, "unhealthy"),
            MyceliumHealth::Unknown => write!(f, "unknown"),
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_disk_size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writable_quota_bytes: Option<u64>,
    /// Disk image format of `source` when it is a virtio-block device
    /// (`raw`, `qcow2`). Set from the storage backend at start time;
    /// absent means the historical raw default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_format: Option<String>,
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
        assert_eq!(mode, NetworkMode::Mycelium);
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
                block_disk_size_bytes: None,
                writable_quota_bytes: None,
                block_format: None,
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
                block_disk_size_bytes: None,
                writable_quota_bytes: None,
                block_format: None,
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
                block_disk_size_bytes: None,
                writable_quota_bytes: None,
                block_format: None,
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

    // RuntimeState schema versioning + transition ring tests

    fn sample_runtime_state() -> RuntimeState {
        RuntimeState {
            id: "vm1".to_string(),
            name: "vm1".to_string(),
            image_ref: "alpine:latest".to_string(),
            image_digest: None,
            vm_config: VmConfig {
                id: "vm1".to_string(),
                name: "vm1".to_string(),
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
                    block_disk_size_bytes: None,
                    writable_quota_bytes: None,
                },
                network: NetworkConfig {
                    mode: NetworkMode::None,
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
            },
            status: VmState::Running,
            schema_version: CURRENT_SCHEMA_VERSION,
            stop_requested: false,
            status_since: None,
            transitions: vec![],
            vmm_pid: 42,
            exit_code: None,
            network_state: None,
            storage_state: None,
            network_ownership: None,
            created_at: Utc::now(),
            started_at: None,
            stopped_at: None,
        }
    }

    #[test]
    fn legacy_state_file_deserializes_with_defaults() {
        // A v1 state.json carries none of the FSM/crash-semantics fields.
        let mut value = serde_json::to_value(sample_runtime_state()).unwrap();
        for key in [
            "schema_version",
            "stop_requested",
            "status_since",
            "transitions",
        ] {
            value.as_object_mut().unwrap().remove(key);
        }
        let legacy_json = serde_json::to_string(&value).unwrap();

        let state: RuntimeState = serde_json::from_str(&legacy_json).unwrap();
        assert_eq!(state.status, VmState::Running);
        assert_eq!(state.schema_version, 1, "absent field means legacy v1");
        assert!(!state.stop_requested);
        assert!(state.status_since.is_none());
        assert!(state.transitions.is_empty());
    }

    #[test]
    fn backfill_schema_version_upgrades_legacy_state() {
        let mut state = sample_runtime_state();
        state.schema_version = 1;
        state.backfill_schema_version();
        assert_eq!(state.schema_version, CURRENT_SCHEMA_VERSION);

        // Already-current state is left alone.
        let mut current = sample_runtime_state();
        current.backfill_schema_version();
        assert_eq!(current.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn new_state_files_carry_current_schema_version() {
        let state = sample_runtime_state();
        let value = serde_json::to_value(&state).unwrap();
        assert_eq!(value["schema_version"], CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn record_transition_appends_history_and_stamps_status_since() {
        let mut state = sample_runtime_state();
        state.status = VmState::Running;

        state.record_transition(VmState::Stopped, "stopped by request");

        assert_eq!(state.status, VmState::Stopped);
        assert_eq!(state.transitions.len(), 1);
        let rec = &state.transitions[0];
        assert_eq!(rec.from, VmState::Running);
        assert_eq!(rec.to, VmState::Stopped);
        assert_eq!(rec.reason, "stopped by request");
        assert!(!rec.at.is_empty());
        assert_eq!(state.status_since.as_deref(), Some(rec.at.as_str()));
    }

    #[test]
    fn transition_ring_caps_at_capacity() {
        let mut state = sample_runtime_state();

        // Exceed the ring capacity twice over; only the most recent
        // TRANSITION_RING_CAPACITY entries survive, in order.
        for i in 0..(TRANSITION_RING_CAPACITY + 10) {
            let to = if i % 2 == 0 {
                VmState::Stopped
            } else {
                VmState::Running
            };
            state.record_transition(to, &format!("cycle {i}"));
        }

        assert_eq!(state.transitions.len(), TRANSITION_RING_CAPACITY);
        assert_eq!(state.transitions.first().unwrap().reason, "cycle 10");
        assert_eq!(state.transitions.last().unwrap().reason, "cycle 41");
        assert_eq!(state.transitions.last().unwrap().to, VmState::Running);
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mycelium_health: Option<MyceliumHealth>,
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
    pub block_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay_backing_dir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkOwnership {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tap_device: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bridge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub iptables_rules: Vec<String>,
}

/// Current on-disk schema version for `RuntimeState` (`state.json`).
///
/// Version 1 is everything written before the FSM/crash-semantics work: no
/// `schema_version`, `stop_requested`, `status_since`, or `transitions`
/// fields. Such files load via `#[serde(default)]` into schema_version 1 and
/// are backfilled to `CURRENT_SCHEMA_VERSION` on the next save.
pub const CURRENT_SCHEMA_VERSION: u32 = 2;

/// Bounded ring-buffer capacity for `RuntimeState::transitions`.
pub const TRANSITION_RING_CAPACITY: usize = 32;

/// One recorded status change: when it happened, which FSM edge was taken and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionRecord {
    /// RFC3339 timestamp of the transition.
    pub at: String,
    pub from: VmState,
    pub to: VmState,
    pub reason: String,
}

/// `schema_version` value a pre-v2 state file deserializes to (field absent).
fn default_legacy_schema_version() -> u32 {
    1
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
    #[serde(default = "default_legacy_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub stop_requested: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_since: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<TransitionRecord>,
    #[serde(default)]
    pub vmm_pid: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_state: Option<NetworkState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage_state: Option<StorageState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_ownership: Option<NetworkOwnership>,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stopped_at: Option<DateTime<Utc>>,
}

impl RuntimeState {
    /// Record a status change in the transition ring and stamp `status_since`.
    ///
    /// The caller must have validated the edge with `fsm::transition_allowed`;
    /// this method only appends history and applies the change.
    pub fn record_transition(&mut self, to: VmState, reason: &str) {
        let at = Utc::now().to_rfc3339();
        self.transitions.push(TransitionRecord {
            at: at.clone(),
            from: self.status,
            to,
            reason: reason.to_string(),
        });
        if self.transitions.len() > TRANSITION_RING_CAPACITY {
            let drop = self.transitions.len() - TRANSITION_RING_CAPACITY;
            self.transitions.drain(0..drop);
        }
        self.status = to;
        self.status_since = Some(at);
    }

    /// Upgrade a state file written by an older chvm in memory; the normal
    /// save path persists the new `schema_version`.
    pub fn backfill_schema_version(&mut self) {
        if self.schema_version < CURRENT_SCHEMA_VERSION {
            self.schema_version = CURRENT_SCHEMA_VERSION;
        }
    }
}
