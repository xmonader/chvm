use crate::cli::{KernelArgs, KernelCommands};
use chvm_lib::kernel::KernelManager;
use chvm_lib::paths::Paths;

pub fn execute(args: &KernelArgs) -> anyhow::Result<()> {
    let paths = Paths::resolve();
    let kernel_mgr = KernelManager::new(&paths.kernels);

    match &args.command {
        KernelCommands::List => {
            let kernels = kernel_mgr.list_kernels()?;
            if kernels.is_empty() {
                println!("No kernels found in {}", paths.kernels.display());
                println!("Run 'chvm kernel extract-host' or 'bash scripts/build-kernel.sh'");
                return Ok(());
            }

            println!(
                "{:<30} {:<12} {:<10} {:<30}",
                "NAME", "SIZE", "DEFAULT", "SYMLINK TARGET"
            );

            let default = paths.kernels.join("vmlinux");
            let default_target = std::fs::read_link(&default).ok();

            for k in &kernels {
                let size = format!("{}MB", k.size / 1_048_576);
                let is_default = if k.name == "vmlinux"
                    || default_target
                        .as_ref()
                        .map(|t| t.to_string_lossy().as_ref() == k.name)
                        .unwrap_or(false)
                {
                    "*"
                } else {
                    ""
                };
                let target = k.symlink_target.as_deref().unwrap_or("-");

                println!(
                    "{:<30} {:<12} {:<10} {:<30}",
                    k.name, size, is_default, target
                );
            }
        }

        KernelCommands::SetDefault(set_args) => {
            kernel_mgr.set_default(&set_args.name)?;
            println!("Default kernel set to: {}", set_args.name);
        }

        KernelCommands::ExtractHost => {
            std::fs::create_dir_all(&paths.kernels)?;
            match kernel_mgr.extract_host_kernel() {
                Ok(path) => {
                    println!("Host kernel extracted to: {path}");

                    // Set as default if no default exists
                    let default = paths.kernels.join("vmlinux");
                    if !default.exists() {
                        kernel_mgr.set_default("vmlinux-host")?;
                        println!("Set as default kernel");
                    }
                }
                Err(e) => {
                    eprintln!("Failed to extract host kernel: {e}");
                    std::process::exit(1);
                }
            }
        }

        KernelCommands::Check(check_args) => {
            let kernel_path = check_args
                .path
                .as_deref()
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| paths.kernels.join("vmlinux"));

            if !kernel_path.exists() {
                eprintln!("Kernel not found: {}", kernel_path.display());
                std::process::exit(1);
            }

            match kernel_mgr.detect_capabilities(&kernel_path) {
                Some(caps) => {
                    println!("Kernel: {}", kernel_path.display());
                    println!(
                        "  IPv6:         {}",
                        if caps.has_ipv6 { "yes" } else { "no" }
                    );
                    println!(
                        "  TUN:          {}",
                        if caps.has_tun { "yes" } else { "no" }
                    );
                    println!(
                        "  VirtioFS:     {}",
                        if caps.has_virtiofs { "yes" } else { "no" }
                    );
                    println!(
                        "  VirtioBlock:  {}",
                        if caps.has_virtio_block { "yes" } else { "no" }
                    );
                    println!(
                        "  Vsock:        {}",
                        if caps.has_vsock { "yes" } else { "no" }
                    );

                    if caps.has_ipv6 && caps.has_tun {
                        println!("\n  Mycelium support: yes");
                    } else {
                        println!("\n  Mycelium support: no (missing IPv6 or TUN)");
                    }
                }
                None => {
                    println!(
                        "Could not detect kernel capabilities for {}",
                        kernel_path.display()
                    );
                    println!("The kernel config may not be embedded.");
                }
            }
        }
    }

    Ok(())
}
