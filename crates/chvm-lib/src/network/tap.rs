use std::collections::HashSet;
use std::process::Command;

use tracing::info;

use crate::network::bridge;
use crate::network::traits::*;
use crate::utils::{run_best_effort, run_cmd};
use crate::vm::state::PortForward;

const DEFAULT_BRIDGE: &str = "chvm0";

/// TAP networking backend.
pub struct TapNetwork;

#[async_trait::async_trait]
impl NetworkBackend for TapNetwork {
    fn name(&self) -> &str {
        "tap"
    }

    async fn setup(&self, config: &SetupConfig) -> anyhow::Result<NetworkResult> {
        let tap_device = format!(
            "tap-{}",
            &config.vm_id[..std::cmp::min(8, config.vm_id.len())]
        );
        let bridge_name = config.bridge.as_deref().unwrap_or(DEFAULT_BRIDGE);

        // Create TAP device
        run_cmd("ip", &["tuntap", "add", "dev", &tap_device, "mode", "tap"])?;

        // Ensure bridge
        bridge::ensure_bridge(bridge_name)?;

        // Add TAP to bridge
        run_cmd("ip", &["link", "set", &tap_device, "master", bridge_name])?;

        // Bring up TAP
        run_cmd("ip", &["link", "set", &tap_device, "up"])?;

        // Prefer the manager-assigned guest IP. Fall back to local discovery only if absent.
        let ip_address = config.ip_address.clone().unwrap_or_else(|| {
            let instance_num = find_unused_ip(bridge_name);
            format!("{}.{}/24", bridge::SUBNET_PREFIX, instance_num)
        });
        let gateway = config
            .gateway
            .clone()
            .unwrap_or_else(|| bridge::GATEWAY.to_string());

        // Setup port forwards
        if !config.port_forwards.is_empty() {
            setup_port_forwards(&ip_address, &config.port_forwards);
        }

        info!(
            tap = %tap_device,
            ip = %ip_address,
            gateway = %gateway,
            "TAP networking configured"
        );

        Ok(NetworkResult {
            tap_device: Some(tap_device),
            mac_address: config.mac_address.clone(),
            ip_address: Some(ip_address),
            gateway: Some(gateway),
            bridge: Some(bridge_name.to_string()),
            mycelium_enabled: false,
            mycelium_peers: Vec::new(),
        })
    }

    async fn teardown(&self, config: &TeardownConfig) -> anyhow::Result<()> {
        // Remove iptables rules for port forwards
        if let Some(ref ip_addr) = config.ip_address {
            remove_port_forwards(ip_addr, &config.port_forwards);
        }

        // Delete TAP device
        let tap_device = format!(
            "tap-{}",
            &config.vm_id[..std::cmp::min(8, config.vm_id.len())]
        );
        run_best_effort("ip", &["link", "delete", &tap_device], "delete TAP device");
        Ok(())
    }
}

/// Find the first unused IP address in the subnet (2-254).
/// Parses IPs currently assigned to TAPs on the bridge to avoid collisions.
fn find_unused_ip(bridge: &str) -> u16 {
    let used = get_used_ips(bridge);
    // .1 is the gateway, start from .2
    for i in 2u16..255 {
        if !used.contains(&i) {
            return i;
        }
    }
    // Fallback (subnet exhausted) — return 2, will likely fail at runtime
    2
}

/// Get the set of IP last-octet values currently in use on TAPs attached to the bridge.
fn get_used_ips(bridge: &str) -> HashSet<u16> {
    let mut used = HashSet::new();
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show"])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            // Also check which devices are on this bridge
            let bridge_devs = get_bridge_devices(bridge);
            for line in stdout.lines() {
                // Format: "N: dev    inet A.B.C.D/M ..."
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    let dev = parts[1].trim_end_matches(':');
                    if bridge_devs.contains(dev) || dev.starts_with("tap-") {
                        // Parse the IP
                        let addr = parts[3];
                        if let Some(ip) = addr.split('/').next() {
                            if ip.starts_with(&format!("{}.", bridge::SUBNET_PREFIX)) {
                                if let Some(last) = ip.rsplit('.').next() {
                                    if let Ok(n) = last.parse::<u16>() {
                                        used.insert(n);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Also count the gateway as used
    used.insert(1);
    used
}

/// Get devices attached to a bridge.
fn get_bridge_devices(bridge: &str) -> HashSet<String> {
    let mut devs = HashSet::new();
    let output = Command::new("ip")
        .args(["link", "show", "master", bridge])
        .output();

    if let Ok(out) = output {
        if out.status.success() {
            let stdout = String::from_utf8_lossy(&out.stdout);
            for line in stdout.lines() {
                // Lines starting with a number have device names
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[0].ends_with(':') {
                    let dev = parts[1].trim_end_matches(':');
                    devs.insert(dev.to_string());
                }
            }
        }
    }
    devs
}

fn setup_port_forwards(vm_ip: &str, forwards: &[PortForward]) {
    run_best_effort(
        "sh",
        &["-c", "echo 1 > /proc/sys/net/ipv4/conf/all/route_localnet"],
        "enable route_localnet",
    );

    let ip = vm_ip.split('/').next().unwrap_or(vm_ip);

    for pf in forwards {
        let proto = if pf.protocol.is_empty() {
            "tcp"
        } else {
            &pf.protocol
        };
        let host_port = pf.host_port.to_string();
        let guest_port = pf.guest_port.to_string();
        let dest = format!("{}:{}", ip, pf.guest_port);

        // DNAT for incoming traffic
        run_best_effort(
            "iptables",
            &[
                "-t",
                "nat",
                "-I",
                "PREROUTING",
                "1",
                "-p",
                proto,
                "--dport",
                &host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &dest,
            ],
            "PREROUTING DNAT rule",
        );

        // DNAT for localhost access
        run_best_effort(
            "iptables",
            &[
                "-t",
                "nat",
                "-A",
                "OUTPUT",
                "-p",
                proto,
                "-d",
                "127.0.0.1",
                "--dport",
                &host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &dest,
            ],
            "OUTPUT DNAT rule",
        );

        // MASQUERADE for hairpin NAT
        run_best_effort(
            "iptables",
            &[
                "-t",
                "nat",
                "-A",
                "POSTROUTING",
                "-p",
                proto,
                "-d",
                ip,
                "--dport",
                &guest_port,
                "-j",
                "MASQUERADE",
            ],
            "POSTROUTING MASQUERADE rule",
        );

        info!(
            host_port = pf.host_port,
            guest_port = pf.guest_port,
            "Port forward configured"
        );
    }
}

fn remove_port_forwards(vm_ip: &str, forwards: &[PortForward]) {
    let ip = vm_ip.split('/').next().unwrap_or(vm_ip);

    for pf in forwards {
        let proto = if pf.protocol.is_empty() {
            "tcp"
        } else {
            &pf.protocol
        };
        let host_port = pf.host_port.to_string();
        let guest_port = pf.guest_port.to_string();
        let dest = format!("{}:{}", ip, pf.guest_port);

        run_best_effort(
            "iptables",
            &[
                "-t",
                "nat",
                "-D",
                "PREROUTING",
                "-p",
                proto,
                "--dport",
                &host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &dest,
            ],
            "remove PREROUTING DNAT rule",
        );

        run_best_effort(
            "iptables",
            &[
                "-t",
                "nat",
                "-D",
                "OUTPUT",
                "-p",
                proto,
                "-d",
                "127.0.0.1",
                "--dport",
                &host_port,
                "-j",
                "DNAT",
                "--to-destination",
                &dest,
            ],
            "remove OUTPUT DNAT rule",
        );

        run_best_effort(
            "iptables",
            &[
                "-t",
                "nat",
                "-D",
                "POSTROUTING",
                "-p",
                proto,
                "-d",
                ip,
                "--dport",
                &guest_port,
                "-j",
                "MASQUERADE",
            ],
            "remove POSTROUTING MASQUERADE rule",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_unused_ip_empty_bridge() {
        let used = HashSet::new();
        let mut used_with_gateway = used.clone();
        used_with_gateway.insert(1);

        let ip = find_unused_ip("nonexistent-bridge");
        assert!((2..255).contains(&ip));
    }

    #[test]
    fn test_get_used_ips_includes_gateway() {
        let used = get_used_ips("nonexistent-bridge");
        assert!(
            used.contains(&1),
            "Gateway IP (.1) should always be marked as used"
        );
    }

    #[test]
    fn test_tap_device_name_format() {
        let vm_id = "test-vm-123456789";
        let expected = "tap-test-vm-";
        let tap_name = format!("tap-{}", &vm_id[..std::cmp::min(8, vm_id.len())]);
        assert!(tap_name.starts_with(expected));
        assert_eq!(tap_name, "tap-test-vm-");
    }

    #[test]
    fn test_tap_device_name_short_id() {
        let vm_id = "short";
        let tap_name = format!("tap-{}", &vm_id[..std::cmp::min(8, vm_id.len())]);
        assert_eq!(tap_name, "tap-short");
    }

    #[test]
    fn test_tap_network_backend_name() {
        let tap = TapNetwork;
        assert_eq!(tap.name(), "tap");
    }

    #[test]
    fn test_default_bridge_constant() {
        assert_eq!(DEFAULT_BRIDGE, "chvm0");
    }
}
