pub mod attach;
pub mod create;
pub mod doctor;
pub mod exec;
pub mod images;
pub mod inspect;
pub mod kernel;
pub mod list;
pub mod login;
pub mod logout;
pub mod logs;
pub mod prune;
pub mod pull;
pub mod rm;
pub mod run;
pub mod setup;
pub mod start;
pub mod stats;
pub mod stop;
pub mod version;

use chvm_lib::oci::{registry_from_image_ref, resolve_credentials};
use chvm_lib::vm::manager::CreateOptions;
use chvm_lib::vm::state::{MountConfig, NetworkMode, PortForward};

/// Common fields shared between RunArgs and CreateArgs.
pub trait CommonVmArgs {
    fn name(&self) -> Option<String>;
    fn image(&self) -> &str;
    fn rootfs(&self) -> Option<String>;
    fn cpus(&self) -> u32;
    fn memory(&self) -> u32;
    fn storage(&self) -> &str;
    fn network(&self) -> &str;
    fn publish(&self) -> &[String];
    fn env(&self) -> &[String];
    fn volume(&self) -> &[String];
    fn kernel(&self) -> Option<String>;
    fn mycelium_peers(&self) -> &[String];
    fn cmd(&self) -> &[String];
}

impl CommonVmArgs for crate::cli::RunArgs {
    fn name(&self) -> Option<String> {
        self.name.clone()
    }
    fn image(&self) -> &str {
        &self.image
    }
    fn rootfs(&self) -> Option<String> {
        self.rootfs.clone()
    }
    fn cpus(&self) -> u32 {
        self.cpus
    }
    fn memory(&self) -> u32 {
        self.memory
    }
    fn storage(&self) -> &str {
        &self.storage
    }
    fn network(&self) -> &str {
        &self.network
    }
    fn publish(&self) -> &[String] {
        &self.publish
    }
    fn env(&self) -> &[String] {
        &self.env
    }
    fn volume(&self) -> &[String] {
        &self.volume
    }
    fn kernel(&self) -> Option<String> {
        self.kernel.clone()
    }
    fn mycelium_peers(&self) -> &[String] {
        &self.mycelium_peers
    }
    fn cmd(&self) -> &[String] {
        &self.cmd
    }
}

impl CommonVmArgs for crate::cli::CreateArgs {
    fn name(&self) -> Option<String> {
        self.name.clone()
    }
    fn image(&self) -> &str {
        &self.image
    }
    fn rootfs(&self) -> Option<String> {
        self.rootfs.clone()
    }
    fn cpus(&self) -> u32 {
        self.cpus
    }
    fn memory(&self) -> u32 {
        self.memory
    }
    fn storage(&self) -> &str {
        &self.storage
    }
    fn network(&self) -> &str {
        &self.network
    }
    fn publish(&self) -> &[String] {
        &self.publish
    }
    fn env(&self) -> &[String] {
        &self.env
    }
    fn volume(&self) -> &[String] {
        &self.volume
    }
    fn kernel(&self) -> Option<String> {
        self.kernel.clone()
    }
    fn mycelium_peers(&self) -> &[String] {
        &self.mycelium_peers
    }
    fn cmd(&self) -> &[String] {
        &self.cmd
    }
}

/// Truncate a string to max length, adding "..." if truncated.
pub fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        format!("{}...", &s[..max - 3])
    } else {
        s.to_string()
    }
}

/// Build CreateOptions from any args implementing CommonVmArgs.
pub fn build_create_options(
    args: &dyn CommonVmArgs,
    detach: bool,
    remove_after: bool,
) -> anyhow::Result<CreateOptions> {
    let port_forwards: Vec<PortForward> = args
        .publish()
        .iter()
        .map(|p| p.parse())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    let mounts: Vec<MountConfig> = args
        .volume()
        .iter()
        .map(|v| v.parse())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    let registry = registry_from_image_ref(args.image());
    let credentials = resolve_credentials(&registry);
    let network_mode = args
        .network()
        .parse::<NetworkMode>()
        .map_err(|e| anyhow::anyhow!("invalid network mode '{}': {e}", args.network()))?;
    let storage_backend = match args.storage() {
        "auto" | "block" | "virtiofs" => args.storage().to_string(),
        other => {
            return Err(anyhow::anyhow!(
                "invalid storage backend '{}': expected auto, block, or virtiofs",
                other
            ));
        }
    };

    Ok(CreateOptions {
        name: args.name(),
        image_ref: if args.rootfs().is_some() {
            None
        } else {
            Some(args.image().to_string())
        },
        rootfs_path: args.rootfs(),
        vcpu: Some(args.cpus()),
        memory_mb: Some(args.memory()),
        storage_backend: Some(storage_backend),
        network_mode: Some(network_mode),
        port_forwards,
        env: args.env().to_vec(),
        cmd: args.cmd().to_vec(),
        mounts,
        kernel_path: args.kernel(),
        detach,
        remove_after,
        mycelium_peers: args.mycelium_peers().to_vec(),
        credentials: Some(credentials),
    })
}
