use nix::mount::{mount, MntFlags, MsFlags};

pub fn mount_rootfs() -> Result<(), Box<dyn std::error::Error>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let mut root_tag = "rootfs".to_string();

    for part in cmdline.split_whitespace() {
        if let Some(tag) = part.strip_prefix("root=") {
            root_tag = tag.to_string();
            break;
        }
    }

    let new_root = "/mnt/rootfs";
    std::fs::create_dir_all(new_root)?;

    println!("[init] Mounting rootfs with tag: {root_tag} at {new_root}");

    mount(
        Some(root_tag.as_str()),
        new_root,
        Some("virtiofs"),
        MsFlags::empty(),
        None::<&str>,
    )?;

    // Verify mount
    let sh_path = format!("{new_root}/bin/sh");
    if !std::path::Path::new(&sh_path).exists() {
        return Err(format!("mounted rootfs but /bin/sh not found at {sh_path}").into());
    }

    println!("[init] Rootfs mounted successfully");
    switch_root(new_root)?;
    println!("[init] Switched root successfully");

    Ok(())
}

fn switch_root(new_root: &str) -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_current_dir(new_root)?;

    // Move essential mounts
    for m in ["/dev", "/proc", "/sys", "/run"] {
        let new_path = format!("{new_root}{m}");
        if let Err(e) = std::fs::create_dir_all(&new_path) {
            eprintln!("[init] Warning: Failed to create {new_path}: {e}");
            continue;
        }
        if let Err(e) = mount(
            Some(m),
            new_path.as_str(),
            None::<&str>,
            MsFlags::MS_MOVE,
            None::<&str>,
        ) {
            eprintln!("[init] Warning: Failed to move mount {m} to {new_path}: {e}");
        }
    }

    // Pivot root
    match nix::unistd::pivot_root(".", "mnt") {
        Ok(()) => {
            if let Err(e) = nix::mount::umount2("/mnt", MntFlags::MNT_DETACH) {
                eprintln!("[init] Warning: Failed to unmount old root /mnt: {e}");
            }
        }
        Err(_) => {
            // Fallback to chroot
            nix::unistd::chroot(".")?;
        }
    }

    std::env::set_current_dir("/")?;
    Ok(())
}
