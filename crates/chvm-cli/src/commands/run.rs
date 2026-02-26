use crate::cli::RunArgs;
use crate::commands::build_create_options;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &RunArgs) -> anyhow::Result<()> {
    let opts = build_create_options(args, args.detach, args.rm)?;

    // Create VM
    let state = manager.create(opts).await?;
    println!("{}", state.id);

    // Start VM
    manager.start(&state.id).await?;

    if args.detach {
        println!("Container {} started in background", state.name);
    } else {
        // Wait for VM to exit
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            // Re-check process liveness before reading status
            if let Err(e) = manager.refresh_status() {
                eprintln!("Warning: Failed to refresh VM status: {e}");
            }
            match manager.get(&state.id) {
                Ok(s) if s.status != chvm_lib::vm::state::VmState::Running => break,
                Err(_) => break,
                _ => {}
            }
        }

        // Print logs
        if let Ok(logs) = manager.logs(&state.id) {
            if !logs.is_empty() {
                print!("{logs}");
            }
        }

        // Remove if --rm
        if args.rm {
            if let Err(e) = manager.remove(&state.id, true).await {
                eprintln!("Warning: Failed to remove VM after exit: {e}");
            }
        }
    }

    Ok(())
}
