use std::process::Command;

pub fn setup_mycelium() -> Result<(), Box<dyn std::error::Error>> {
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
        None => return Ok(()),
    };

    println!("[init] Starting mycelium with peers: {peers}");

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

    let child = Command::new(&mycelium_bin)
        .args(&args)
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()?;

    println!("[init] Mycelium started (PID {})", child.id());

    // Wait for interface
    std::thread::sleep(std::time::Duration::from_secs(3));

    Ok(())
}
