use crate::cli::StartArgs;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &StartArgs) -> anyhow::Result<()> {
    manager.start(&args.container).await?;
    println!("{}", args.container);
    Ok(())
}
