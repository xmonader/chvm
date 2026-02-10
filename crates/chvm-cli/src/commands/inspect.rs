use crate::cli::InspectArgs;
use chvm_lib::vm::VmManager;

pub fn execute(manager: &VmManager, args: &InspectArgs) -> anyhow::Result<()> {
    let state = manager.get(&args.container)?;
    let json = serde_json::to_string_pretty(&state)?;
    println!("{json}");
    Ok(())
}
