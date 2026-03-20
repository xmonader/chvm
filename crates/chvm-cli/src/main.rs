mod cli;
mod commands;

use clap::{CommandFactory, Parser};
use clap_complete::CompleteEnv;
use tracing_subscriber::EnvFilter;

use chvm_lib::config::Config;
use chvm_lib::paths::Paths;
use chvm_lib::vm::VmManager;

use cli::{Cli, Commands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    CompleteEnv::with_factory(Cli::command).complete();

    let cli = Cli::parse();

    // Setup logging
    let filter = if cli.verbose {
        "chvm=debug,chvm_lib=debug"
    } else {
        "chvm=info,chvm_lib=info"
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(filter)),
        )
        .with_target(false)
        .without_time()
        .init();

    // Handle commands that don't need a full manager
    match &cli.command {
        Commands::Version(args) => return commands::version::execute(args),
        Commands::Doctor(args) => return commands::doctor::execute(args).await,
        Commands::Setup(args) => return commands::setup::execute(args),
        Commands::Kernel(args) => return commands::kernel::execute(args),
        Commands::Login(args) => return commands::login::execute(args),
        Commands::Logout(args) => return commands::logout::execute(args),
        _ => {}
    }

    // Initialize paths and config
    let paths = Paths::resolve();
    paths.ensure_dirs()?;

    let config = Config::load(&paths.config_file)?;
    let mut manager = VmManager::new(config, paths)?;

    match &cli.command {
        Commands::Pull(args) => commands::pull::execute(&mut manager, args).await,
        Commands::Run(args) => commands::run::execute(&mut manager, args).await,
        Commands::Create(args) => commands::create::execute(&mut manager, args).await,
        Commands::Start(args) => commands::start::execute(&mut manager, args).await,
        Commands::Stop(args) => commands::stop::execute(&mut manager, args).await,
        Commands::Rm(args) => commands::rm::execute(&mut manager, args).await,
        Commands::Exec(args) => commands::exec::execute(&manager, args).await,
        Commands::Attach(args) => commands::attach::execute(&manager, args).await,
        Commands::List(args) => commands::list::execute(&mut manager, args).await,
        Commands::Logs(args) => commands::logs::execute(&mut manager, args),
        Commands::Inspect(args) => commands::inspect::execute(&mut manager, args).await,
        Commands::Stats(args) => commands::stats::execute(&mut manager, args).await,
        Commands::Images(args) => commands::images::execute(&mut manager, args),
        Commands::Rmi(args) => commands::images::execute_remove(&mut manager, args),
        Commands::Prune(args) => commands::prune::execute(&mut manager, args).await,
        // Already handled above
        Commands::Version(_)
        | Commands::Doctor(_)
        | Commands::Setup(_)
        | Commands::Kernel(_)
        | Commands::Login(_)
        | Commands::Logout(_) => {
            unreachable!()
        }
    }
}
