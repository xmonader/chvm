use crate::cli::StatsArgs;
use chvm_lib::vm::VmManager;

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

pub async fn execute(manager: &mut VmManager, args: &StatsArgs) -> anyhow::Result<()> {
    let (state, counters) = manager.stats(&args.container).await?;

    let short_id = &state.vm_config.id[..12.min(state.vm_config.id.len())];

    if args.json {
        let mut network = serde_json::json!({
            "mode": state.vm_config.network.mode.to_string(),
        });
        if let Some(ns) = &state.network_state {
            if let Some(ip) = &ns.ip_address {
                network["ip_address"] = serde_json::json!(ip);
            }
            if let Some(gw) = &ns.gateway {
                network["gateway"] = serde_json::json!(gw);
            }
            if ns.mycelium_enabled {
                network["mycelium_ip"] = serde_json::json!(ns.mycelium_ip);
            }
        }
        if let Some(net_counters) = counters.get("net") {
            network["counters"] = net_counters.clone();
        }

        let obj = serde_json::json!({
            "id": state.vm_config.id,
            "name": state.vm_config.name,
            "vcpu": state.vm_config.vcpu,
            "memory_mb": state.vm_config.memory_mb,
            "network": network,
            "counters": counters,
        });

        println!("{}", serde_json::to_string_pretty(&obj)?);
        return Ok(());
    }

    // Human-readable output
    println!("Container: {} ({})", state.vm_config.name, short_id);
    println!();

    // CPU
    println!("CPU");
    println!("  vCPUs: {}", state.vm_config.vcpu);
    println!();

    // Memory
    println!("MEMORY");
    println!("  Allocated: {} MB", state.vm_config.memory_mb);
    println!();

    // Network
    println!("NETWORK");
    println!("  Mode: {}", state.vm_config.network.mode);
    if let Some(ns) = &state.network_state {
        if let Some(ip) = &ns.ip_address {
            println!("  IP Address: {ip}");
        }
        if let Some(gw) = &ns.gateway {
            println!("  Gateway: {gw}");
        }
        if ns.mycelium_enabled {
            if let Some(myc_ip) = &ns.mycelium_ip {
                println!("  Mycelium IP: {myc_ip}");
            }
        }
    }
    if let Some(net) = counters.get("net").and_then(|v| v.as_object()) {
        for (iface, stats) in net {
            if let Some(stats_obj) = stats.as_object() {
                let rx = stats_obj
                    .get("rx_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let tx = stats_obj
                    .get("tx_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                println!("  {iface}:");
                println!("    RX: {}", format_bytes(rx));
                println!("    TX: {}", format_bytes(tx));
            }
        }
    }
    println!();

    // Block I/O
    if let Some(block) = counters.get("block").and_then(|v| v.as_object()) {
        println!("BLOCK I/O");
        for (dev, stats) in block {
            if let Some(stats_obj) = stats.as_object() {
                let read = stats_obj
                    .get("read_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let write = stats_obj
                    .get("write_bytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                println!("  {dev}:");
                println!("    Read:  {}", format_bytes(read));
                println!("    Write: {}", format_bytes(write));
            }
        }
        println!();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn test_format_bytes_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1023), "1023 B");
    }

    #[test]
    fn test_format_bytes_kilobytes() {
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1536), "1.50 KB");
    }

    #[test]
    fn test_format_bytes_megabytes() {
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.00 MB");
    }

    #[test]
    fn test_format_bytes_gigabytes() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.00 GB");
    }
}
