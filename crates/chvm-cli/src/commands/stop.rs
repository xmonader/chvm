use crate::cli::StopArgs;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &StopArgs) -> anyhow::Result<()> {
    manager.stop(&args.container).await?;
    println!("{}", args.container);
    Ok(())
}
