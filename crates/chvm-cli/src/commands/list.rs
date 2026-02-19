use crate::cli::ListArgs;
use crate::commands::truncate;
use chvm_lib::vm::state::VmState;
use chvm_lib::vm::VmManager;

pub fn execute(manager: &mut VmManager, args: &ListArgs) -> anyhow::Result<()> {
    let _ = manager.refresh_status();
    let vms = manager.list();

    // Print header
    println!(
        "{:<14} {:<20} {:<30} {:<10} {:<24} {:<6} {:<8}",
        "CONTAINER ID", "NAME", "IMAGE", "STATUS", "CREATED", "CPUS", "MEMORY"
    );

    for vm in &vms {
        if !args.all && vm.status != VmState::Running {
            continue;
        }

        let created = vm.created_at.format("%Y-%m-%d %H:%M:%S").to_string();
        let memory = format!("{}MB", vm.vm_config.memory_mb);

        println!(
            "{:<14} {:<20} {:<30} {:<10} {:<24} {:<6} {:<8}",
            &vm.id[..std::cmp::min(12, vm.id.len())],
            &vm.name,
            truncate(&vm.image_ref, 28),
            vm.status,
            created,
            vm.vm_config.vcpu,
            memory,
        );
    }

    Ok(())
}
