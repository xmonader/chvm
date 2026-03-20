use crate::cli::VersionArgs;

pub fn execute(_args: &VersionArgs) -> anyhow::Result<()> {
    println!("chvm version {}", env!("CHVM_FULL_VERSION"));
    println!("Cloud Hypervisor MicroVM Manager");

    // Show CH version if available
    if let Some(ch_path) = chvm_lib::hypervisor::process::find_ch_binary() {
        let output = std::process::Command::new(&ch_path)
            .arg("--version")
            .output();
        if let Ok(output) = output {
            let version = String::from_utf8_lossy(&output.stdout);
            println!("Cloud Hypervisor: {}", version.trim());
        }
    }

    Ok(())
}
