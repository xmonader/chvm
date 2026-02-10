use crate::cli::PruneArgs;
use chvm_lib::vm::VmManager;

pub async fn execute(manager: &mut VmManager, _args: &PruneArgs) -> anyhow::Result<()> {
    let pruned = manager.prune().await?;
    if pruned.is_empty() {
        println!("No stopped containers to prune");
    } else {
        for id in &pruned {
            println!("Deleted: {id}");
        }
        println!("Total reclaimed: {} containers", pruned.len());
    }
    Ok(())
}
