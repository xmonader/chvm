use crate::cli::InspectArgs;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &InspectArgs) -> anyhow::Result<()> {
    // Run mycelium health check for running VMs before displaying state.
    if let Err(e) = manager.check_mycelium_health(&args.container).await {
        eprintln!("Warning: mycelium health check failed: {e}");
    }

    let state = manager.get(&args.container)?;
    let json = serde_json::to_string_pretty(&state)?;
    println!("{json}");
    Ok(())
}
