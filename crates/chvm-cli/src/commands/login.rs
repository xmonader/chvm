use std::io::{self, BufRead, Write};

use crate::cli::LoginArgs;
use chvm_lib::oci::{DockerConfig, docker_config_path};

pub fn execute(args: &LoginArgs) -> anyhow::Result<()> {
    let username = match &args.username {
        Some(u) => u.clone(),
        None => {
            print!("Username: ");
            io::stdout().flush()?;
            let mut line = String::new();
            io::stdin().lock().read_line(&mut line)?;
            line.trim().to_string()
        }
    };

    if username.is_empty() {
        anyhow::bail!("Username cannot be empty");
    }

    let password = if args.password_stdin {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        line.trim().to_string()
    } else {
        match &args.password {
            Some(p) => p.clone(),
            None => rpassword::prompt_password_stdout("Password: ")?,
        }
    };

    if password.is_empty() {
        anyhow::bail!("Password cannot be empty");
    }

    let config_path = docker_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine Docker config path"))?;

    let mut config = if config_path.exists() {
        DockerConfig::load(&config_path)?
    } else {
        DockerConfig::default()
    };

    config.set_auth(&args.server, &username, &password);
    config.save(&config_path)?;

    println!("Login Succeeded");
    println!("Credentials stored in {}", config_path.display());
    Ok(())
}
