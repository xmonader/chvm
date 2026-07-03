use crate::cli::PruneArgs;
use chvm_lib::vm::state::VmState;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, args: &PruneArgs) -> anyhow::Result<()> {
    let stopped = manager
        .list()
        .iter()
        .filter(|s| s.status == VmState::Stopped)
        .count();

    if stopped == 0 {
        println!("No stopped containers to prune");
        return Ok(());
    }

    // Prune irreversibly deletes each container's writable data. Guard the
    // interactive case with a confirmation; in non-interactive use (scripts,
    // pipes) proceed, since there's no TTY to prompt on — pass --yes to be
    // explicit.
    use std::io::IsTerminal;
    if !args.yes
        && std::io::stdin().is_terminal()
        && !confirm(&format!("Delete {stopped} stopped container(s)? [y/N] "))?
    {
        println!("Aborted.");
        return Ok(());
    }

    let pruned = manager.prune().await?;
    for id in &pruned {
        println!("Deleted: {id}");
    }
    println!("Total reclaimed: {} containers", pruned.len());
    Ok(())
}

fn confirm(prompt: &str) -> anyhow::Result<bool> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(matches!(input.trim().to_lowercase().as_str(), "y" | "yes"))
}
