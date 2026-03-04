use std::process::{Command, Stdio};

/// Configuration needed to spawn and supervise the mycelium daemon.
pub struct MyceliumConfig {
    pub binary: String,
    pub args: Vec<String>,
}

/// Parse kernel cmdline for mycelium configuration and return a `MyceliumConfig`
/// if mycelium peers are configured. Does not spawn the process.
pub fn setup_mycelium() -> Result<Option<MyceliumConfig>, Box<dyn std::error::Error>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let mut peers = None;
    let mut key_path = None;

    for part in cmdline.split_whitespace() {
        if let Some(val) = part.strip_prefix("chvm.mycelium_peer=") {
            peers = Some(val.to_string());
        }
        if let Some(val) = part.strip_prefix("chvm.mycelium_key=") {
            key_path = Some(val.to_string());
        }
    }

    let peers = match peers {
        Some(p) => p,
        None => return Ok(None),
    };

    let mycelium_bin =
        crate::helpers::find_binary("mycelium", &[]).ok_or("mycelium binary not found")?;

    std::fs::create_dir_all("/etc/mycelium")?;
    let key = key_path.unwrap_or_else(|| "/etc/mycelium/priv_key.bin".to_string());

    let mut args = vec![
        "--key-file".to_string(),
        key,
        "--tun-name".to_string(),
        "mycelium".to_string(),
        "--disable-quic".to_string(),
    ];

    for peer in peers.split(',') {
        let peer = peer.trim();
        if !peer.is_empty() {
            args.push("--peers".into());
            args.push(peer.to_string());
        }
    }

    println!("[init] Mycelium configured with peers: {peers}");

    Ok(Some(MyceliumConfig {
        binary: mycelium_bin,
        args,
    }))
}

/// Spawn the mycelium daemon, returning its PID and Child handle on success.
pub fn spawn_mycelium(config: &MyceliumConfig) -> Option<(i32, std::process::Child)> {
    println!("[init] Starting mycelium...");

    let child = Command::new(&config.binary)
        .args(&config.args)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| eprintln!("[init] Failed to start mycelium: {e}"))
        .ok()?;

    let pid = child.id() as i32;
    println!("[init] Mycelium started (PID {pid})");
    Some((pid, child))
}
