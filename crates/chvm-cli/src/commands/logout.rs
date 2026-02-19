use crate::cli::LogoutArgs;
use chvm_lib::oci::{DockerConfig, docker_config_path};

pub fn execute(args: &LogoutArgs) -> anyhow::Result<()> {
    let config_path = docker_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine Docker config path"))?;

    if !config_path.exists() {
        println!("Not logged in to {}", args.server);
        return Ok(());
    }

    let mut config = DockerConfig::load(&config_path)?;

    if config.remove_auth(&args.server) {
        config.save(&config_path)?;
        println!("Removed login credentials for {}", args.server);
    } else {
        println!("Not logged in to {}", args.server);
    }

    Ok(())
}
