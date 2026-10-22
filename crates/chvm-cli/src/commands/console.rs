use std::path::Path;

use crate::cli::ConsoleArgs;
use chvm_lib::console;
use chvm_lib::vm::state::VmState;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &ConsoleArgs) -> anyhow::Result<()> {
    let state = manager.get(&args.container)?;
    if state.status != VmState::Running {
        anyhow::bail!("container '{}' is not running", args.container);
    }

    let api_socket = state
        .vm_config
        .api_socket_path()
        .ok_or_else(|| anyhow::anyhow!("container '{}' has no API socket path", args.container))?;

    let pty_path = console::serial_pty_path(Path::new(&api_socket))
        .await
        .map_err(|e| anyhow::anyhow!("cannot reach serial console of '{}': {e}", args.container))?;

    console::console(&pty_path).await?;
    Ok(())
}
