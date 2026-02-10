use crate::cli::ExecArgs;
use chvm_lib::vm::manager::ExecOptions;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &VmManager, args: &ExecArgs) -> anyhow::Result<()> {
    // Strip leading "--" that clap's `last = true` may capture
    let cmd: Vec<String> = if args.cmd.first().map(|s| s.as_str()) == Some("--") {
        args.cmd[1..].to_vec()
    } else {
        args.cmd.clone()
    };

    if cmd.is_empty() {
        anyhow::bail!("no command specified");
    }

    let opts = ExecOptions {
        cmd,
        env: args.env.clone(),
        working_dir: args.workdir.clone(),
    };

    let exit_code = manager.exec(&args.container, opts).await?;
    std::process::exit(exit_code);
}
