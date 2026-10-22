use crate::cli::EventsArgs;
use chvm_lib::vm::VmManager;

pub fn execute(manager: &mut VmManager, args: &EventsArgs) -> anyhow::Result<()> {
    let events = manager.vm_events(&args.container)?;
    if events.is_empty() {
        println!("No transitions recorded for '{}'", args.container);
        return Ok(());
    }

    println!("{:<28} {:<10} {:<10} REASON", "TIME", "FROM", "TO");
    for event in &events {
        println!(
            "{:<28} {:<10} {:<10} {}",
            event.at, event.from, event.to, event.reason
        );
    }
    Ok(())
}
