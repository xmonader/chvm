use std::collections::HashSet;
use std::process::Command;

use crate::cli::DoctorArgs;
use chvm_lib::config::Config;
use chvm_lib::hypervisor::process::find_ch_binary;
use chvm_lib::network::bridge;
use chvm_lib::paths::Paths;
use chvm_lib::vm::state::VmState;
use chvm_lib::vm::VmManager;

pub async fn execute(args: &DoctorArgs) -> anyhow::Result<()> {
    println!("CHVM Doctor - System Check\n");

    let mut all_ok = true;

    // Check Cloud Hypervisor
    print!("Cloud Hypervisor: ");
    match find_ch_binary() {
        Some(path) => {
            let version = Command::new(&path)
                .arg("--version")
                .output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| "unknown version".into());
            println!("OK ({path}) - {version}");
        }
        None => {
            println!("NOT FOUND");
            println!("  Install: https://github.com/cloud-hypervisor/cloud-hypervisor/releases");
            all_ok = false;
        }
    }

    // Check virtiofsd
    print!("virtiofsd: ");
    let virtiofsd_paths = [
        "virtiofsd",
        "/usr/libexec/virtiofsd",
        "/usr/lib/virtiofsd",
        "/usr/local/bin/virtiofsd",
    ];
    let found = virtiofsd_paths
        .iter()
        .any(|p| which::which(p).is_ok() || std::path::Path::new(p).exists());
    if found {
        println!("OK");
    } else {
        println!("NOT FOUND (optional, needed for virtiofs storage)");
    }

    // Check kernel
    let paths = Paths::resolve();
    print!("Kernel: ");
    let default_kernel = paths.kernels.join("vmlinux");
    if default_kernel.exists() {
        let size = std::fs::metadata(&default_kernel)
            .map(|m| format!("{}MB", m.len() / 1_048_576))
            .unwrap_or_else(|_| "?".into());
        println!("OK ({}) - {size}", default_kernel.display());
    } else {
        let kernel_count = std::fs::read_dir(&paths.kernels)
            .map(|e| e.count())
            .unwrap_or(0);
        if kernel_count > 0 {
            println!(
                "WARN - No default kernel symlink, but {} kernels found",
                kernel_count
            );
        } else {
            println!("NOT FOUND");
            println!("  Run: chvm kernel extract-host");
            all_ok = false;
        }
    }

    // Check mkfs.ext4
    print!("mkfs.ext4: ");
    if which::which("mkfs.ext4").is_ok() {
        println!("OK");
    } else {
        println!("NOT FOUND (needed for block storage)");
        all_ok = false;
    }

    // Check iptables
    print!("iptables: ");
    if which::which("iptables").is_ok() {
        println!("OK");
    } else {
        println!("NOT FOUND (needed for TAP networking)");
    }

    // Check ip command
    print!("ip command: ");
    if which::which("ip").is_ok() {
        println!("OK");
    } else {
        println!("NOT FOUND (needed for networking)");
        all_ok = false;
    }

    // Check /dev/kvm
    print!("/dev/kvm: ");
    if std::path::Path::new("/dev/kvm").exists() {
        println!("OK");
    } else {
        println!("NOT FOUND");
        println!("  Enable KVM: modprobe kvm_intel (or kvm_amd)");
        all_ok = false;
    }

    // Check data directory
    print!("Data directory: ");
    if paths.base.exists() {
        println!("OK ({})", paths.base.display());
    } else {
        println!("Will be created at {}", paths.base.display());
    }

    println!();
    if all_ok {
        println!("All checks passed! CHVM is ready to use.");
    } else {
        println!("Some checks failed. Please install missing dependencies.");
    }

    if args.gc {
        println!("\n--- Garbage Collection ---\n");
        gc_cleanup().await?;
    }

    Ok(())
}

async fn gc_cleanup() -> anyhow::Result<()> {
    let paths = Paths::resolve();
    paths.ensure_dirs()?;
    let config = Config::load(&paths.config_file)?;
    let mut manager = VmManager::new(config, paths)?;

    // Level 1: Detect crashed VMs and clean up their resources via refresh_status
    let running_before: Vec<String> = manager
        .list()
        .iter()
        .filter(|s| s.status == VmState::Running)
        .map(|s| s.id.clone())
        .collect();

    manager.refresh_status().await?;

    let running_after: Vec<String> = manager
        .list()
        .iter()
        .filter(|s| s.status == VmState::Running)
        .map(|s| s.id.clone())
        .collect();

    let cleaned = running_before.len() - running_after.len();
    if cleaned > 0 {
        println!("Cleaned up {} crashed VM(s)", cleaned);
    } else {
        println!("No crashed VMs found");
    }

    // Level 2: Detect orphaned TAP devices and iptables rules
    let running_prefixes: HashSet<String> = running_after
        .iter()
        .map(|id| id[..std::cmp::min(8, id.len())].to_string())
        .collect();

    // Collect IPs of running VMs for iptables orphan detection
    let running_ips: HashSet<String> = manager
        .list()
        .iter()
        .filter(|s| s.status == VmState::Running)
        .filter_map(|s| {
            s.network_state
                .as_ref()
                .and_then(|ns| ns.ip_address.as_ref())
                .map(|ip| ip.split('/').next().unwrap_or(ip).to_string())
        })
        .collect();

    gc_orphaned_tap_devices(&running_prefixes);
    gc_orphaned_iptables_rules(&running_ips);

    Ok(())
}

fn gc_orphaned_tap_devices(running_prefixes: &HashSet<String>) {
    let output = match Command::new("ip").args(["-o", "link", "show"]).output() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("Warning: Failed to list interfaces: {e}");
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut orphaned = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Look for interfaces named like "tap-XXXXXXXX" where XXXXXXXX is a VM prefix
        if parts.len() >= 2 {
            let dev = parts[1] // get the interface name (e.g. "2: tap-abcdef12@...")
                .trim_end_matches(':')
                .split('@')
                .next()
                .unwrap_or("");
            if let Some(prefix) = dev.strip_prefix("tap-") {
                if !running_prefixes.contains(prefix) {
                    orphaned.push(dev.to_string());
                }
            }
        }
    }

    if orphaned.is_empty() {
        println!("No orphaned TAP devices found");
    } else {
        for dev in &orphaned {
            print!("Removing orphaned TAP device: {dev} ... ");
            match Command::new("ip").args(["link", "delete", dev]).output() {
                Ok(out) if out.status.success() => println!("done"),
                Ok(out) => println!("failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
                Err(e) => println!("failed: {e}"),
            }
        }
    }
}

fn gc_orphaned_iptables_rules(running_ips: &HashSet<String>) {
    let output = match Command::new("iptables").args(["-t", "nat", "-S"]).output() {
        Ok(out) => out,
        Err(e) => {
            eprintln!("Warning: Failed to list iptables rules: {e}");
            return;
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut removed = 0;

    for line in stdout.lines() {
        if !line.contains(bridge::SUBNET_PREFIX) {
            continue;
        }
        // Skip the bridge-level rules (subnet-wide MASQUERADE/FORWARD, not per-VM)
        if line.contains(bridge::DEFAULT_SUBNET) {
            continue;
        }
        if line.starts_with("-P ") {
            continue;
        }

        // Extract the per-VM IP from the rule (e.g. 192.168.200.2)
        let ip = extract_chvm_ip(line);
        if let Some(ref ip) = ip {
            if running_ips.contains(ip) {
                continue; // belongs to a running VM
            }
        }

        let delete_rule = line.replacen("-A ", "-D ", 1).replacen("-I ", "-D ", 1);
        let args: Vec<&str> = delete_rule.split_whitespace().collect();

        let mut cmd_args = vec!["-t", "nat"];
        cmd_args.extend_from_slice(&args);

        match Command::new("iptables").args(&cmd_args).output() {
            Ok(out) if out.status.success() => removed += 1,
            _ => {}
        }
    }

    if removed > 0 {
        println!("Removed {} orphaned iptables rule(s)", removed);
    } else {
        println!("No orphaned iptables rules found");
    }
}

/// Extract a chvm-subnet IP (192.168.200.X) from an iptables rule line.
/// Looks for --to-destination or -d flags containing the subnet prefix.
fn extract_chvm_ip(line: &str) -> Option<String> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        // --to-destination 192.168.200.X:PORT or --to-destination 192.168.200.X
        if *part == "--to-destination" {
            if let Some(val) = parts.get(i + 1) {
                let ip = val.split(':').next().unwrap_or(val);
                if ip.starts_with(bridge::SUBNET_PREFIX) {
                    return Some(ip.to_string());
                }
            }
        }
        // -d 192.168.200.X
        if *part == "-d" {
            if let Some(val) = parts.get(i + 1) {
                if val.starts_with(bridge::SUBNET_PREFIX) {
                    return Some(val.to_string());
                }
            }
        }
    }
    None
}
