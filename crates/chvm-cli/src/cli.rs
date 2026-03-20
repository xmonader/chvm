use clap::{Parser, Subcommand};
use clap_complete::engine::{ArgValueCompleter, CompletionCandidate};

fn complete_vm(current: &std::ffi::OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return vec![];
    };

    let paths = chvm_lib::paths::Paths::resolve();
    let entries = match std::fs::read_dir(&paths.vms) {
        Ok(e) => e,
        Err(_) => return vec![],
    };

    let mut candidates = vec![];
    for entry in entries.flatten() {
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        let vm_id = entry.file_name().to_string_lossy().to_string();
        let state_file = entry.path().join("state.json");
        let name = std::fs::read_to_string(&state_file)
            .ok()
            .and_then(|data| serde_json::from_str::<serde_json::Value>(&data).ok())
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(String::from));

        // Offer the VM ID if it prefix-matches
        if vm_id.starts_with(current) {
            let mut c = CompletionCandidate::new(vm_id.clone());
            if let Some(ref n) = name {
                c = c.help(Some(n.clone().into()));
            }
            candidates.push(c);
        }

        // Offer the VM name if it prefix-matches
        if let Some(ref n) = name {
            if n.starts_with(current) && n != &vm_id {
                candidates.push(CompletionCandidate::new(n.as_str()).help(Some(vm_id.into())));
            }
        }
    }
    candidates
}

#[derive(Parser)]
#[command(
    name = "chvm",
    about = "Cloud Hypervisor MicroVM Manager",
    version = env!("CHVM_FULL_VERSION")
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    /// Enable verbose output
    #[arg(long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Pull an image from a registry
    Pull(PullArgs),

    /// Create and start a container
    Run(RunArgs),

    /// Create a container without starting it
    Create(CreateArgs),

    /// Start a stopped container
    Start(StartArgs),

    /// Stop a running container
    Stop(StopArgs),

    /// Remove a container
    Rm(RmArgs),

    /// Execute a command in a running container
    Exec(ExecArgs),

    /// Attach to a running container
    Attach(AttachArgs),

    /// List containers
    #[command(alias = "ps", alias = "ls")]
    List(ListArgs),

    /// Show container logs
    Logs(LogsArgs),

    /// Show detailed container information
    Inspect(InspectArgs),

    /// Show resource usage statistics for a container
    Stats(StatsArgs),

    /// List images
    Images(ImagesArgs),

    /// Remove an image
    Rmi(ImageRmArgs),

    /// Remove stopped containers
    Prune(PruneArgs),

    /// Check system requirements
    Doctor(DoctorArgs),

    /// Setup system dependencies
    Setup(SetupArgs),

    /// Manage kernels
    Kernel(KernelArgs),

    /// Show version information
    Version(VersionArgs),

    /// Log in to a container registry
    Login(LoginArgs),

    /// Log out from a container registry
    Logout(LogoutArgs),
}

#[derive(clap::Args)]
pub struct PullArgs {
    /// Image reference (e.g., alpine:latest)
    pub image: String,
}

#[derive(clap::Args)]
pub struct RunArgs {
    /// Container name
    #[arg(long)]
    pub name: Option<String>,

    /// Number of vCPUs
    #[arg(long, default_value = "1")]
    pub cpus: u32,

    /// Memory in MB
    #[arg(long, default_value = "512")]
    pub memory: u32,

    /// Storage backend override (block, virtiofs)
    #[arg(long)]
    pub storage: Option<String>,

    /// Network mode (tap, mycelium, none)
    #[arg(long, default_value = "mycelium")]
    pub network: String,

    /// Publish ports (HOST:GUEST)
    #[arg(short, long)]
    pub publish: Vec<String>,

    /// Environment variables (KEY=VALUE)
    #[arg(short, long)]
    pub env: Vec<String>,

    /// Volume mounts (HOST:GUEST[:ro])
    #[arg(short = 'v', long)]
    pub volume: Vec<String>,

    /// Kernel path
    #[arg(long)]
    pub kernel: Option<String>,

    /// Run in background
    #[arg(short, long)]
    pub detach: bool,

    /// Remove container when it exits
    #[arg(long)]
    pub rm: bool,

    /// Local rootfs directory
    #[arg(long)]
    pub rootfs: Option<String>,

    /// Mycelium peer addresses
    #[arg(long)]
    pub mycelium_peers: Vec<String>,

    /// Image reference
    pub image: String,

    /// Command to run
    pub cmd: Vec<String>,
}

#[derive(clap::Args)]
pub struct CreateArgs {
    /// Container name
    #[arg(long)]
    pub name: Option<String>,

    #[arg(long, default_value = "1")]
    pub cpus: u32,

    #[arg(long, default_value = "512")]
    pub memory: u32,

    /// Storage backend override (block, virtiofs)
    #[arg(long)]
    pub storage: Option<String>,

    #[arg(long, default_value = "mycelium")]
    pub network: String,

    #[arg(short, long)]
    pub publish: Vec<String>,

    #[arg(short, long)]
    pub env: Vec<String>,

    #[arg(short = 'v', long)]
    pub volume: Vec<String>,

    #[arg(long)]
    pub kernel: Option<String>,

    #[arg(long)]
    pub rootfs: Option<String>,

    #[arg(long)]
    pub mycelium_peers: Vec<String>,

    pub image: String,

    pub cmd: Vec<String>,
}

#[derive(clap::Args)]
pub struct StartArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,
}

#[derive(clap::Args)]
pub struct StopArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,
}

#[derive(clap::Args)]
pub struct RmArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,

    /// Force removal of running container
    #[arg(short, long)]
    pub force: bool,
}

#[derive(clap::Args)]
#[command(trailing_var_arg = true)]
pub struct ExecArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,

    /// Command to execute
    #[arg(allow_hyphen_values = true)]
    pub cmd: Vec<String>,

    /// Environment variables
    #[arg(short, long)]
    pub env: Vec<String>,

    /// Working directory
    #[arg(short, long)]
    pub workdir: Option<String>,
}

#[derive(clap::Args)]
pub struct AttachArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,
}

#[derive(clap::Args)]
pub struct ListArgs {
    /// Show all containers (default shows only running)
    #[arg(short, long)]
    pub all: bool,
}

#[derive(clap::Args)]
pub struct LogsArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,

    /// Follow log output
    #[arg(short, long)]
    pub follow: bool,
}

#[derive(clap::Args)]
pub struct InspectArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,

    /// Run mycelium health check before displaying state
    #[arg(long)]
    pub check_mycelium_health: bool,
}

#[derive(clap::Args)]
pub struct StatsArgs {
    /// Container ID or name
    #[arg(add = ArgValueCompleter::new(complete_vm))]
    pub container: String,

    /// Output in JSON format
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct ImagesArgs {
    #[command(subcommand)]
    pub command: Option<ImageCommands>,
}

#[derive(Subcommand)]
pub enum ImageCommands {
    /// Remove a cached image by reference or digest
    #[command(alias = "remove")]
    Rm(ImageRmArgs),
}

#[derive(clap::Args, Clone)]
pub struct ImageRmArgs {
    /// Image reference (e.g. ubuntu:24.04) or digest (e.g. sha256:...)
    pub image: String,
}

#[derive(clap::Args)]
pub struct PruneArgs {}

#[derive(clap::Args)]
pub struct DoctorArgs {
    /// Clean up leaked TAP devices, iptables rules, and dead VM resources
    #[arg(long)]
    pub gc: bool,
}

#[derive(clap::Args)]
pub struct SetupArgs {
    /// What to setup (all, kernel, network)
    #[arg(default_value = "all")]
    pub component: String,
}

#[derive(clap::Args)]
pub struct KernelArgs {
    #[command(subcommand)]
    pub command: KernelCommands,
}

#[derive(Subcommand)]
pub enum KernelCommands {
    /// List available kernels
    List,
    /// Set default kernel
    SetDefault(KernelSetDefaultArgs),
    /// Extract host kernel
    ExtractHost,
    /// Check kernel capabilities
    Check(KernelCheckArgs),
}

#[derive(clap::Args)]
pub struct KernelSetDefaultArgs {
    /// Kernel name
    pub name: String,
}

#[derive(clap::Args)]
pub struct KernelCheckArgs {
    /// Kernel path (defaults to default kernel)
    pub path: Option<String>,
}

#[derive(clap::Args)]
pub struct VersionArgs {}

#[derive(clap::Args)]
pub struct LoginArgs {
    /// Registry server (default: docker.io)
    #[arg(default_value = "docker.io")]
    pub server: String,

    /// Username
    #[arg(short, long)]
    pub username: Option<String>,

    /// Password
    #[arg(short, long)]
    pub password: Option<String>,

    /// Read password from stdin
    #[arg(long)]
    pub password_stdin: bool,
}

#[derive(clap::Args)]
pub struct LogoutArgs {
    /// Registry server (default: docker.io)
    #[arg(default_value = "docker.io")]
    pub server: String,
}
