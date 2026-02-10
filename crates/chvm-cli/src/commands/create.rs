use crate::cli::CreateArgs;
use crate::commands::build_create_options;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &CreateArgs) -> anyhow::Result<()> {
    let opts = build_create_options(args, false, false)?;
    let state = manager.create(opts).await?;
    println!("{}", state.id);
    Ok(())
}
