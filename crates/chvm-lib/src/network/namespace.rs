use crate::utils::{run_cmd, run_best_effort};

/// Create a network namespace.
pub fn create_netns(name: &str) -> anyhow::Result<()> {
    run_cmd("ip", &["netns", "add", name])
}

/// Delete a network namespace.
pub fn delete_netns(name: &str) -> anyhow::Result<()> {
    run_best_effort("ip", &["netns", "delete", name], "delete netns");
    Ok(())
}

/// Create a veth pair.
pub fn create_veth(name: &str, peer: &str) -> anyhow::Result<()> {
    run_cmd("ip", &["link", "add", name, "type", "veth", "peer", "name", peer])
}

/// Move an interface into a network namespace.
pub fn move_to_netns(iface: &str, netns: &str) -> anyhow::Result<()> {
    run_cmd("ip", &["link", "set", iface, "netns", netns])
}
