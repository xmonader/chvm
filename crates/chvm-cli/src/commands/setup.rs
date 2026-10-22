use crate::cli::SetupArgs;
use chvm_lib::paths::Paths;

pub fn execute(args: &SetupArgs) -> anyhow::Result<()> {
    let paths = Paths::resolve();

    match args.component.as_str() {
        "all" => {
            println!("Setting up CHVM...\n");

            // Create directories
            paths.ensure_dirs()?;
            println!("Created directories at {}", paths.base.display());

            // Grant the chvm group read access to state (docs/security.md)
            setup_access_group(&paths);

            // Try to download/extract kernel
            println!("\nKernel setup:");
            let kernel_mgr = chvm_lib::kernel::KernelManager::new(&paths.kernels);
            match kernel_mgr.extract_host_kernel() {
                Ok(path) => println!("  Extracted host kernel to {path}"),
                Err(e) => println!("  Could not extract host kernel: {e}"),
            }

            // Set default kernel if available
            let default = paths.kernels.join("vmlinux");
            if !default.exists() {
                if let Ok(kernels) = kernel_mgr.list_kernels() {
                    if let Some(k) = kernels.first() {
                        match kernel_mgr.set_default(&k.name) {
                            Ok(()) => println!("  Set default kernel: {}", k.name),
                            Err(e) => eprintln!("  Warning: Failed to set default kernel: {e}"),
                        }
                    }
                }
            }

            println!("\nSetup complete! Run 'chvm doctor' to verify.");
            println!("\nFor best performance, build a custom kernel:");
            println!("  bash scripts/build-kernel.sh");
            println!("  chvm kernel set-default vmlinux-chvm-6.6.70");
        }
        "kernel" => {
            paths.ensure_dirs()?;
            let kernel_mgr = chvm_lib::kernel::KernelManager::new(&paths.kernels);
            match kernel_mgr.extract_host_kernel() {
                Ok(path) => println!("Extracted host kernel to {path}"),
                Err(e) => println!("Could not extract host kernel: {e}"),
            }
        }
        "network" => {
            println!("Network setup is automatic on first VM start.");
        }
        other => {
            anyhow::bail!("unknown component: {other}. Use 'all', 'kernel', or 'network'.");
        }
    }

    Ok(())
}

/// Create the `chvm` access group (if missing) and grant it read access to
/// the state directory so read-only commands work without root. Best-effort:
/// prints guidance instead of failing when groupadd/chown are unavailable.
fn setup_access_group(paths: &Paths) {
    let group = chvm_lib::permissions::ACCESS_GROUP;
    let created = std::process::Command::new("groupadd")
        .args(["-f", group])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    let base = paths.base.to_string_lossy().to_string();
    let owner = format!("root:{group}");
    let granted = created
        && std::process::Command::new("chown")
            .args([&owner, &base])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        && std::process::Command::new("chmod")
            .args(["0750", &base])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

    if granted {
        println!("\nAccess group '{group}' ready.");
        println!("Grant read-only access (list/inspect/logs/stats/images/events) with:");
        println!("  sudo usermod -aG {group} <user>");
    } else {
        eprintln!("  Warning: could not configure the '{group}' access group (needs root)");
    }
}
