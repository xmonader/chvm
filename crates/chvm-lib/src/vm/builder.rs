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
        let network_mode = self.network_mode.unwrap_or_else(|| {
            config.default_network_mode.parse().unwrap_or_default()
        });

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
        let working_dir = self.image_config.as_ref()
            .and_then(|ic| if ic.working_dir.is_empty() { None } else { Some(ic.working_dir.clone()) })
            .unwrap_or_else(|| "/".into());

        // Build kernel cmdline
        let mut cmdline_builder = crate::hypervisor::cmdline::KernelCmdlineBuilder::new()
            .console("hvc0")
            .rw(true)
            .init("/chvm-init");

        match self.rootfs_type {
            RootfsType::VirtioFs => {
                cmdline_builder = cmdline_builder
                    .root("rootfs")
                    .raw("rootfstype=virtiofs");
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
