use crate::cli::AttachArgs;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &VmManager, args: &AttachArgs) -> anyhow::Result<()> {
    let exit_code = manager.attach(&args.container).await?;
    std::process::exit(exit_code);
}
