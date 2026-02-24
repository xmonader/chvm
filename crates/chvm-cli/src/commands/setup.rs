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
