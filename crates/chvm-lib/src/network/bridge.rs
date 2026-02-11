use std::process::Command;

use tracing::info;

use crate::utils::{run_best_effort, run_cmd};

pub const DEFAULT_BRIDGE_IP: &str = "192.168.200.1/24";
pub const DEFAULT_SUBNET: &str = "192.168.200.0/24";
/// Prefix used when assigning VM IPs on the bridge subnet.
pub const SUBNET_PREFIX: &str = "192.168.200";
/// Gateway IP for VMs on the bridge subnet.
pub const GATEWAY: &str = "192.168.200.1";

/// Ensure a bridge exists and is configured.
pub fn ensure_bridge(name: &str) -> anyhow::Result<()> {
    // Check if bridge already exists
    let output = Command::new("ip").args(["link", "show", name]).output()?;

    if output.status.success() {
        return Ok(());
    }

    info!(bridge = name, "Creating bridge");

    run_cmd("ip", &["link", "add", name, "type", "bridge"])?;
    run_cmd("ip", &["link", "set", name, "up"])?;

    // Assign IP (may already exist)
    run_best_effort(
        "ip",
        &["addr", "add", DEFAULT_BRIDGE_IP, "dev", name],
        "assign bridge IP",
    );

    // Enable IP forwarding
    run_best_effort(
        "sh",
        &["-c", "echo 1 > /proc/sys/net/ipv4/ip_forward"],
        "enable IP forwarding",
    );

    // Setup NAT
    setup_nat(name);

    Ok(())
}

fn setup_nat(bridge: &str) {
    // Enable route_localnet for localhost port forwarding
    run_best_effort(
        "sh",
        &["-c", "echo 1 > /proc/sys/net/ipv4/conf/all/route_localnet"],
        "enable route_localnet",
    );

    // MASQUERADE (idempotent: check before adding)
    ensure_iptables_rule(&[
        "-t",
        "nat",
        "POSTROUTING",
        "-s",
        DEFAULT_SUBNET,
        "!",
        "-o",
        bridge,
        "-j",
        "MASQUERADE",
    ]);

    // Allow forwarding (insert at top for priority over Docker's DOCKER-FORWARD chain)
    ensure_iptables_rule_insert(&["-i", bridge, "-j", "ACCEPT"]);
    ensure_iptables_rule_insert(&["-o", bridge, "-j", "ACCEPT"]);
}

/// Check if an iptables rule exists (-C), add it (-A) if not.
fn ensure_iptables_rule(args: &[&str]) {
    let mut check_args: Vec<&str> = args.to_vec();
    // Replace the table position with -C for checking
    if let Some(pos) = check_args
        .iter()
        .position(|&a| a == "POSTROUTING" || a == "FORWARD")
    {
        check_args.insert(pos, "-C");
    }

    let exists = Command::new("iptables")
        .args(&check_args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !exists {
        let mut add_args: Vec<&str> = args.to_vec();
        if let Some(pos) = add_args
            .iter()
            .position(|&a| a == "POSTROUTING" || a == "FORWARD")
        {
            add_args.insert(pos, "-A");
        }
        run_best_effort("iptables", &add_args, "add iptables rule");
    }
}

/// Check if a FORWARD rule exists, insert at position 1 if not.
fn ensure_iptables_rule_insert(args: &[&str]) {
    let mut check_args = vec!["-C", "FORWARD"];
    check_args.extend_from_slice(args);

    let exists = Command::new("iptables")
        .args(&check_args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !exists {
        let mut insert_args = vec!["-I", "FORWARD", "1"];
        insert_args.extend_from_slice(args);
        run_best_effort("iptables", &insert_args, "insert iptables FORWARD rule");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bridge_constants() {
        assert_eq!(DEFAULT_BRIDGE_IP, "192.168.200.1/24");
        assert_eq!(DEFAULT_SUBNET, "192.168.200.0/24");
        assert_eq!(SUBNET_PREFIX, "192.168.200");
        assert_eq!(GATEWAY, "192.168.200.1");
    }

    #[test]
    fn test_subnet_prefix_matches_gateway() {
        assert!(GATEWAY.starts_with(SUBNET_PREFIX));
    }

    #[test]
    fn test_default_bridge_ip_format() {
        assert!(DEFAULT_BRIDGE_IP.contains("/24"));
        assert!(DEFAULT_BRIDGE_IP.starts_with(SUBNET_PREFIX));
    }

    #[test]
    fn test_default_subnet_format() {
        assert!(DEFAULT_SUBNET.ends_with(".0/24"));
        assert!(DEFAULT_SUBNET.starts_with(SUBNET_PREFIX));
    }
}
