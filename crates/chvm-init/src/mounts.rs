use nix::mount::{mount, MsFlags};

pub fn mount_essential() -> Result<(), Box<dyn std::error::Error>> {
    let mounts = [
        ("proc", "/proc", "proc", MsFlags::empty(), ""),
        ("sysfs", "/sys", "sysfs", MsFlags::empty(), ""),
        ("devtmpfs", "/dev", "devtmpfs", MsFlags::empty(), ""),
        (
            "devpts",
            "/dev/pts",
            "devpts",
            MsFlags::empty(),
            "gid=5,mode=620",
        ),
        ("tmpfs", "/tmp", "tmpfs", MsFlags::empty(), ""),
        ("tmpfs", "/run", "tmpfs", MsFlags::empty(), ""),
    ];

    for (source, target, fstype, flags, data) in mounts {
        if let Err(e) = std::fs::create_dir_all(target) {
            eprintln!("[init] Warning: Failed to create mount target {target}: {e}");
        }

        let data_opt: Option<&str> = if data.is_empty() { None } else { Some(data) };
        match mount(Some(source), target, Some(fstype), flags, data_opt) {
            Ok(()) => {}
            Err(nix::errno::Errno::EBUSY) => {} // Already mounted
            Err(e) => eprintln!("[init] Warning: Failed to mount {target}: {e}"),
        }
    }

    // Create symlinks
    let _ = std::os::unix::fs::symlink("/proc/self/fd", "/dev/fd");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/0", "/dev/stdin");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/1", "/dev/stdout");
    let _ = std::os::unix::fs::symlink("/proc/self/fd/2", "/dev/stderr");

    Ok(())
}

pub fn mount_volumes() -> Result<(), Box<dyn std::error::Error>> {
    let cmdline = match std::fs::read_to_string("/proc/cmdline") {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };

    for part in cmdline.split_whitespace() {
        if let Some(mounts_str) = part.strip_prefix("chvm.mounts=") {
            for m in mounts_str.split(',') {
                if let Some((tag, target)) = m.split_once('=') {
                    println!("[init] Mounting volume {tag} at {target}");
                    if let Err(e) = std::fs::create_dir_all(target) {
                        eprintln!("[init] Warning: Failed to create mount target {target}: {e}");
                    }
                    match mount(
                        Some(tag),
                        target,
                        Some("virtiofs"),
                        MsFlags::empty(),
                        None::<&str>,
                    ) {
                        Ok(()) => {}
                        Err(e) => eprintln!(
                            "[init] Warning: Failed to mount volume {tag} at {target}: {e}"
                        ),
                    }
                }
            }
        }
    }

    Ok(())
}
