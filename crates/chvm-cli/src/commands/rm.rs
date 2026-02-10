use crate::cli::RmArgs;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &RmArgs) -> anyhow::Result<()> {
    manager.remove(&args.container, args.force).await?;
    println!("{}", args.container);
    Ok(())
}
