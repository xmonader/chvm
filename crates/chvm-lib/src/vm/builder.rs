use crate::config::Config;
use crate::oci::ImageConfig;
use crate::vm::state::*;

/// VmBuilder merges OCI image config with CLI arguments to produce a VmConfig.
pub struct VmBuilder {
    name: String,
    id: String,
    image_config: Option<ImageConfig>,
    vcpu: Option<u32>,
    memory_mb: Option<u32>,
    network_mode: Option<NetworkMode>,
    storage_backend: Option<String>,
    block_disk_size_bytes: Option<u64>,
    writable_quota_bytes: Option<u64>,
    kernel_path: Option<String>,
    cmd: Vec<String>,
    env: Vec<String>,
    port_forwards: Vec<PortForward>,
    mounts: Vec<MountConfig>,
    mycelium_peers: Vec<String>,
    rootfs_source: String,
    rootfs_type: RootfsType,
    vsock_cid: u32,
    state_dir: String,
    log_path: String,
}

impl VmBuilder {
    pub fn new(id: String, name: String) -> Self {
        Self {
            name,
            id,
            image_config: None,
            vcpu: None,
            memory_mb: None,
            network_mode: None,
            storage_backend: None,
            block_disk_size_bytes: None,
            writable_quota_bytes: None,
            kernel_path: None,
            cmd: Vec::new(),
            env: Vec::new(),
            port_forwards: Vec::new(),
            mounts: Vec::new(),
            mycelium_peers: Vec::new(),
            rootfs_source: String::new(),
            rootfs_type: RootfsType::VirtioBlock,
            vsock_cid: 0,
            state_dir: String::new(),
            log_path: String::new(),
        }
    }

    pub fn image_config(mut self, config: ImageConfig) -> Self {
        self.image_config = Some(config);
        self
    }

    pub fn vcpu(mut self, vcpu: u32) -> Self {
        self.vcpu = Some(vcpu);
        self
    }

    pub fn memory_mb(mut self, mb: u32) -> Self {
        self.memory_mb = Some(mb);
        self
    }

    pub fn network_mode(mut self, mode: NetworkMode) -> Self {
        self.network_mode = Some(mode);
        self
    }

    pub fn storage_backend(mut self, backend: String) -> Self {
        self.storage_backend = Some(backend);
        self
    }

    pub fn block_disk_size_bytes(mut self, size: u64) -> Self {
        self.block_disk_size_bytes = Some(size);
        self
    }

    pub fn writable_quota_bytes(mut self, size: u64) -> Self {
        self.writable_quota_bytes = Some(size);
        self
    }

    pub fn kernel_path(mut self, path: String) -> Self {
        self.kernel_path = Some(path);
        self
    }

    pub fn cmd(mut self, cmd: Vec<String>) -> Self {
        self.cmd = cmd;
        self
    }

    pub fn env(mut self, env: Vec<String>) -> Self {
        self.env = env;
        self
    }

    pub fn port_forwards(mut self, pf: Vec<PortForward>) -> Self {
        self.port_forwards = pf;
        self
    }

    pub fn mounts(mut self, mounts: Vec<MountConfig>) -> Self {
        self.mounts = mounts;
        self
    }

    pub fn mycelium_peers(mut self, peers: Vec<String>) -> Self {
        self.mycelium_peers = peers;
        self
    }

    pub fn rootfs(mut self, source: String, rootfs_type: RootfsType) -> Self {
        self.rootfs_source = source;
        self.rootfs_type = rootfs_type;
        self
    }

    pub fn vsock_cid(mut self, cid: u32) -> Self {
        self.vsock_cid = cid;
        self
    }

    pub fn state_dir(mut self, dir: String) -> Self {
        self.state_dir = dir;
        self
    }

    pub fn log_path(mut self, path: String) -> Self {
        self.log_path = path;
        self
    }

    pub fn build(self, config: &Config) -> VmConfig {
        let vcpu = self.vcpu.unwrap_or(config.default_vcpu);
        let memory_mb = self.memory_mb.unwrap_or(config.default_memory_mb);
        let network_mode = self
            .network_mode
            .unwrap_or_else(|| config.default_network_mode.parse().unwrap_or_default());

        // Merge command: CLI cmd > image entrypoint+cmd > /bin/sh
        let cmd = if !self.cmd.is_empty() {
            self.cmd
        } else if let Some(ref ic) = self.image_config {
            let mut merged = Vec::new();
            if !ic.entrypoint.is_empty() {
                merged.extend(ic.entrypoint.clone());
                merged.extend(ic.cmd.clone());
            } else {
                merged.extend(ic.cmd.clone());
            }
            if merged.is_empty() {
                vec!["/bin/sh".into()]
            } else {
                merged
            }
        } else {
            vec!["/bin/sh".into()]
        };

        // Merge env: image env + CLI env (CLI overrides)
        let mut env = Vec::new();
        if let Some(ref ic) = self.image_config {
            env.extend(ic.env.clone());
        }
        env.extend(self.env);

        // Working directory
        let working_dir = self
            .image_config
            .as_ref()
            .and_then(|ic| {
                if ic.working_dir.is_empty() {
                    None
                } else {
                    Some(ic.working_dir.clone())
                }
            })
            .unwrap_or_else(|| "/".into());

        // Build kernel cmdline
        let mut cmdline_builder = crate::hypervisor::cmdline::KernelCmdlineBuilder::new()
            .console("hvc0")
            .rw(true)
            .init("/chvm-init");

        match self.rootfs_type {
            RootfsType::VirtioFs => {
                cmdline_builder = cmdline_builder.root("rootfs").raw("rootfstype=virtiofs");
            }
            RootfsType::VirtioBlock => {
                cmdline_builder = cmdline_builder.root("/dev/vda");
            }
        }

        let kernel_cmdline = cmdline_builder
            .cmd(&cmd)
            .env(&env)
            .workdir(&working_dir)
            .build();

        let mac = generate_mac();

        let (block_disk_size_bytes, writable_quota_bytes) = match self.rootfs_type {
            RootfsType::VirtioBlock => (
                self.block_disk_size_bytes
                    .or(config.default_block_disk_size_mb.map(|mb| mb * 1024 * 1024)),
                None,
            ),
            RootfsType::VirtioFs => (
                None,
                self.writable_quota_bytes
                    .or(config.default_virtiofs_quota_mb.map(|mb| mb * 1024 * 1024)),
            ),
        };

        VmConfig {
            id: self.id,
            name: self.name,
            vcpu,
            memory_mb,
            kernel: KernelConfig {
                path: self.kernel_path.unwrap_or_default(),
                cmdline: kernel_cmdline,
                initrd_path: None,
            },
            rootfs: RootfsConfig {
                source: self.rootfs_source,
                rootfs_type: self.rootfs_type,
                read_only: false,
                block_disk_size_bytes,
                writable_quota_bytes,
            },
            network: NetworkConfig {
                mode: network_mode,
                tap_device: None,
                mac_address: Some(mac),
                ip_address: None,
                gateway: None,
                port_forwards: self.port_forwards,
                mycelium_peers: self.mycelium_peers,
                mycelium_key: None,
            },
            init: InitConfig {
                cmd,
                env,
                working_dir: Some(working_dir),
            },
            mounts: self.mounts,
            log_path: Some(self.log_path),
            vsock_cid: self.vsock_cid,
            state_dir: Some(self.state_dir),
        }
    }
}

/// Generate a random MAC address with the QEMU/KVM prefix 52:54:00.
pub fn generate_mac() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let b: [u8; 3] = [rng.gen(), rng.gen(), rng.gen()];
    format!("52:54:00:{:02x}:{:02x}:{:02x}", b[0], b[1], b[2])
}

/// Generate a unique vsock CID from a VM ID string.
/// CIDs 0, 1, 2 are reserved (hypervisor, local, host).
/// Uses FNV-1a hash for better distribution over the full 32-bit CID space.
pub fn generate_vsock_cid(vm_id: &str) -> u32 {
    // FNV-1a hash — much better distribution than simple multiply-add
    let mut hash: u32 = 0x811c9dc5; // FNV offset basis
    for b in vm_id.bytes() {
        hash ^= b as u32;
        hash = hash.wrapping_mul(0x01000193); // FNV prime
    }
    // Map to valid CID range: 3..=(u32::MAX - 1)
    // u32::MAX (0xFFFFFFFF) is VMADDR_CID_ANY, so avoid it too
    let range = u32::MAX - 3; // 4294967292 possible values
    (hash % range) + 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_generate_mac_format() {
        let mac = generate_mac();
        assert!(mac.starts_with("52:54:00:"));
        let parts: Vec<&str> = mac.split(':').collect();
        assert_eq!(parts.len(), 6);
        assert_eq!(parts[0], "52");
        assert_eq!(parts[1], "54");
        assert_eq!(parts[2], "00");
    }

    #[test]
    fn test_generate_mac_uniqueness() {
        let mac1 = generate_mac();
        let mac2 = generate_mac();
        assert_ne!(mac1, mac2);
    }

    #[test]
    fn test_generate_vsock_cid_deterministic() {
        let cid1 = generate_vsock_cid("test-vm-123");
        let cid2 = generate_vsock_cid("test-vm-123");
        assert_eq!(cid1, cid2);
    }

    #[test]
    fn test_generate_vsock_cid_different_ids() {
        let cid1 = generate_vsock_cid("vm-1");
        let cid2 = generate_vsock_cid("vm-2");
        assert_ne!(cid1, cid2);
    }

    #[test]
    fn test_generate_vsock_cid_reserved_range() {
        let cid = generate_vsock_cid("test-vm");
        assert!(cid >= 3, "CID must be >= 3 (reserved: 0, 1, 2)");
        assert!(cid < u32::MAX, "CID must be < u32::MAX (VMADDR_CID_ANY)");
    }

    #[test]
    fn test_generate_vsock_cid_distribution() {
        let mut cids = std::collections::HashSet::new();
        for i in 0..100 {
            let cid = generate_vsock_cid(&format!("vm-{}", i));
            cids.insert(cid);
        }
        assert!(
            cids.len() > 95,
            "Should have good distribution, got {} unique CIDs",
            cids.len()
        );
    }

    #[test]
    fn test_vm_builder_new() {
        let builder = VmBuilder::new("test-id".to_string(), "test-vm".to_string());
        assert_eq!(builder.id, "test-id");
        assert_eq!(builder.name, "test-vm");
        assert!(builder.image_config.is_none());
        assert!(builder.vcpu.is_none());
        assert!(builder.memory_mb.is_none());
    }

    #[test]
    fn test_vm_builder_with_vcpu() {
        let builder = VmBuilder::new("id".to_string(), "name".to_string()).vcpu(4);
        assert_eq!(builder.vcpu, Some(4));
    }

    #[test]
    fn test_vm_builder_with_memory() {
        let builder = VmBuilder::new("id".to_string(), "name".to_string()).memory_mb(2048);
        assert_eq!(builder.memory_mb, Some(2048));
    }

    #[test]
    fn test_vm_builder_with_network_mode() {
        let builder = VmBuilder::new("id".to_string(), "name".to_string())
            .network_mode(NetworkMode::Mycelium);
        assert_eq!(builder.network_mode, Some(NetworkMode::Mycelium));
    }

    #[test]
    fn test_vm_builder_build_defaults() {
        let config = Config::default();
        let builder = VmBuilder::new("test-id".to_string(), "test-vm".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.id, "test-id");
        assert_eq!(vm_config.name, "test-vm");
        assert_eq!(vm_config.vcpu, 1);
        assert_eq!(vm_config.memory_mb, 512);
        assert_eq!(vm_config.network.mode, NetworkMode::Mycelium);
    }

    #[test]
    fn test_vm_builder_build_with_overrides() {
        let config = Config::default();
        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .vcpu(4)
            .memory_mb(2048)
            .network_mode(NetworkMode::None)
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(10)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.vcpu, 4);
        assert_eq!(vm_config.memory_mb, 2048);
        assert_eq!(vm_config.network.mode, NetworkMode::None);
    }

    #[test]
    fn test_vm_builder_cmd_merge_cli_priority() {
        let config = Config::default();
        let image_config = ImageConfig {
            cmd: vec!["image-cmd".to_string()],
            entrypoint: vec!["image-entrypoint".to_string()],
            env: vec![],
            working_dir: String::new(),
            user: String::new(),
            labels: HashMap::new(),
        };

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .image_config(image_config)
            .cmd(vec!["cli-cmd".to_string()])
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.init.cmd, vec!["cli-cmd"]);
    }

    #[test]
    fn test_vm_builder_cmd_merge_image_entrypoint_and_cmd() {
        let config = Config::default();
        let image_config = ImageConfig {
            cmd: vec!["arg1".to_string(), "arg2".to_string()],
            entrypoint: vec!["/entrypoint.sh".to_string()],
            env: vec![],
            working_dir: String::new(),
            user: String::new(),
            labels: HashMap::new(),
        };

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .image_config(image_config)
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.init.cmd, vec!["/entrypoint.sh", "arg1", "arg2"]);
    }

    #[test]
    fn test_vm_builder_cmd_fallback_to_shell() {
        let config = Config::default();
        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.init.cmd, vec!["/bin/sh"]);
    }

    #[test]
    fn test_vm_builder_env_merge() {
        let config = Config::default();
        let image_config = ImageConfig {
            cmd: vec![],
            entrypoint: vec![],
            env: vec!["IMAGE_VAR=image".to_string()],
            working_dir: String::new(),
            user: String::new(),
            labels: HashMap::new(),
        };

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .image_config(image_config)
            .env(vec!["CLI_VAR=cli".to_string()])
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.init.env, vec!["IMAGE_VAR=image", "CLI_VAR=cli"]);
    }

    #[test]
    fn test_vm_builder_working_dir_from_image() {
        let config = Config::default();
        let image_config = ImageConfig {
            cmd: vec![],
            entrypoint: vec![],
            env: vec![],
            working_dir: "/app".to_string(),
            user: String::new(),
            labels: HashMap::new(),
        };

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .image_config(image_config)
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.init.working_dir, Some("/app".to_string()));
    }

    #[test]
    fn test_vm_builder_working_dir_default() {
        let config = Config::default();
        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.init.working_dir, Some("/".to_string()));
    }

    #[test]
    fn test_vm_builder_rootfs_types() {
        let config = Config::default();

        let builder_block = VmBuilder::new("id1".to_string(), "vm1".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());
        let vm_block = builder_block.build(&config);
        assert!(vm_block.kernel.cmdline.contains("/dev/vda"));

        let builder_fs = VmBuilder::new("id2".to_string(), "vm2".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioFs)
            .vsock_cid(4)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());
        let vm_fs = builder_fs.build(&config);
        assert!(vm_fs.kernel.cmdline.contains("rootfs"));
        assert!(vm_fs.kernel.cmdline.contains("virtiofs"));
    }

    #[test]
    fn test_vm_builder_only_persists_size_settings_for_selected_backend() {
        let config = Config {
            default_block_disk_size_mb: Some(2048),
            default_virtiofs_quota_mb: Some(4096),
            ..Config::default()
        };

        let vm_block = VmBuilder::new("id1".to_string(), "vm1".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string())
            .writable_quota_bytes(1234)
            .build(&config);
        assert_eq!(
            vm_block.rootfs.block_disk_size_bytes,
            Some(2048 * 1024 * 1024)
        );
        assert_eq!(vm_block.rootfs.writable_quota_bytes, None);

        let vm_fs = VmBuilder::new("id2".to_string(), "vm2".to_string())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioFs)
            .vsock_cid(4)
            .state_dir("/state".to_string())
            .log_path("/log".to_string())
            .block_disk_size_bytes(5678)
            .build(&config);
        assert_eq!(vm_fs.rootfs.block_disk_size_bytes, None);
        assert_eq!(vm_fs.rootfs.writable_quota_bytes, Some(4096 * 1024 * 1024));
    }

    #[test]
    fn test_vm_builder_port_forwards() {
        let config = Config::default();
        let pf = PortForward {
            host_port: 8080,
            guest_port: 80,
            protocol: "tcp".to_string(),
        };

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .port_forwards(vec![pf.clone()])
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.network.port_forwards.len(), 1);
        assert_eq!(vm_config.network.port_forwards[0].host_port, 8080);
    }

    #[test]
    fn test_vm_builder_mounts() {
        let config = Config::default();
        let mount = MountConfig {
            source: "/host".to_string(),
            target: "/guest".to_string(),
            mount_type: None,
            read_only: false,
            tag: None,
        };

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .mounts(vec![mount.clone()])
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.mounts.len(), 1);
        assert_eq!(vm_config.mounts[0].source, "/host");
    }

    #[test]
    fn test_vm_builder_mycelium_peers() {
        let config = Config::default();
        let peers = vec!["tcp://1.2.3.4:9651".to_string()];

        let builder = VmBuilder::new("id".to_string(), "vm".to_string())
            .mycelium_peers(peers.clone())
            .rootfs("/rootfs".to_string(), RootfsType::VirtioBlock)
            .vsock_cid(3)
            .state_dir("/state".to_string())
            .log_path("/log".to_string());

        let vm_config = builder.build(&config);
        assert_eq!(vm_config.network.mycelium_peers, peers);
    }
}
