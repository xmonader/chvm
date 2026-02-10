use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "chvm", about = "Cloud Hypervisor MicroVM Manager", version)]
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

    /// List images
    Images(ImagesArgs),

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

    /// Storage backend (block, virtiofs)
    #[arg(long, default_value = "block")]
    pub storage: String,

    /// Network mode (tap, mycelium, none)
    #[arg(long, default_value = "tap")]
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

    #[arg(long, default_value = "block")]
    pub storage: String,

    #[arg(long, default_value = "tap")]
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
    pub container: String,
}

#[derive(clap::Args)]
pub struct StopArgs {
    /// Container ID or name
    pub container: String,
}

#[derive(clap::Args)]
pub struct RmArgs {
    /// Container ID or name
    pub container: String,

    /// Force removal of running container
    #[arg(short, long)]
    pub force: bool,
}

#[derive(clap::Args)]
pub struct ExecArgs {
    /// Container ID or name
    pub container: String,

    /// Command to execute
    #[arg(last = true)]
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
    pub container: String,

    /// Follow log output
    #[arg(short, long)]
    pub follow: bool,
}

#[derive(clap::Args)]
pub struct InspectArgs {
    /// Container ID or name
    pub container: String,
}

#[derive(clap::Args)]
pub struct ImagesArgs {}

#[derive(clap::Args)]
pub struct PruneArgs {}

#[derive(clap::Args)]
pub struct DoctorArgs {}

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
