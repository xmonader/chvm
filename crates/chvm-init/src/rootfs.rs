use nix::mount::{mount, MntFlags, MsFlags};

#[derive(Debug, Clone, PartialEq, Eq)]
struct RootSpec {
    source: String,
    fstype: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MountInfo {
    source: String,
    target: String,
    fstype: String,
}

fn parse_root_spec(cmdline: &str) -> RootSpec {
    let mut source = "/dev/vda".to_string();
    let mut fstype = None;

    for part in cmdline.split_whitespace() {
        if let Some(root) = part.strip_prefix("root=") {
            source = root.to_string();
        } else if let Some(kind) = part.strip_prefix("rootfstype=") {
            fstype = Some(kind.to_string());
        }
    }

    RootSpec { source, fstype }
}

fn current_root_mount() -> Option<MountInfo> {
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let source = fields.next()?;
        let target = fields.next()?;
        let fstype = fields.next()?;
        if target == "/" {
            return Some(MountInfo {
                source: source.to_string(),
                target: target.to_string(),
                fstype: fstype.to_string(),
            });
        }
    }
    None
}

fn needs_root_switch(current: &MountInfo, desired: &RootSpec) -> bool {
    if desired.source == "rootfs" || desired.fstype.as_deref() == Some("virtiofs") {
        return !(current.source == "rootfs" && current.fstype == "virtiofs");
    }

    current.source != desired.source
}

pub fn should_mount_rootfs() -> bool {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let desired = parse_root_spec(&cmdline);

    match current_root_mount() {
        Some(current) => needs_root_switch(&current, &desired),
        None => true,
    }
}

pub fn mount_rootfs() -> Result<(), Box<dyn std::error::Error>> {
    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let root = parse_root_spec(&cmdline);

    let new_root = "/mnt/rootfs";
    std::fs::create_dir_all(new_root)?;

    let fstype = if root.source == "rootfs" || root.fstype.as_deref() == Some("virtiofs") {
        "virtiofs"
    } else {
        root.fstype.as_deref().unwrap_or("ext4")
    };

    println!(
        "[init] Mounting rootfs source {} type {} at {}",
        root.source, fstype, new_root
    );

    mount(
        Some(root.source.as_str()),
        new_root,
        Some(fstype),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_root_spec_defaults_to_block_root() {
        let spec = parse_root_spec("console=hvc0 rw init=/chvm-init");
        assert_eq!(
            spec,
            RootSpec {
                source: "/dev/vda".into(),
                fstype: None,
            }
        );
    }

    #[test]
    fn parse_root_spec_reads_virtiofs_root() {
        let spec = parse_root_spec("root=rootfs rootfstype=virtiofs init=/chvm-init");
        assert_eq!(
            spec,
            RootSpec {
                source: "rootfs".into(),
                fstype: Some("virtiofs".into()),
            }
        );
    }

    #[test]
    fn block_root_does_not_switch_when_already_on_disk_root() {
        let current = MountInfo {
            source: "/dev/vda".into(),
            target: "/".into(),
            fstype: "ext4".into(),
        };
        let desired = RootSpec {
            source: "/dev/vda".into(),
            fstype: None,
        };
        assert!(!needs_root_switch(&current, &desired));
    }

    #[test]
    fn block_root_switches_from_initramfs() {
        let current = MountInfo {
            source: "rootfs".into(),
            target: "/".into(),
            fstype: "rootfs".into(),
        };
        let desired = RootSpec {
            source: "/dev/vda".into(),
            fstype: None,
        };
        assert!(needs_root_switch(&current, &desired));
    }

    #[test]
    fn virtiofs_root_switches_until_real_root_is_mounted() {
        let current = MountInfo {
            source: "tmpfs".into(),
            target: "/".into(),
            fstype: "tmpfs".into(),
        };
        let desired = RootSpec {
            source: "rootfs".into(),
            fstype: Some("virtiofs".into()),
        };
        assert!(needs_root_switch(&current, &desired));

        let mounted = MountInfo {
            source: "rootfs".into(),
            target: "/".into(),
            fstype: "virtiofs".into(),
        };
        assert!(!needs_root_switch(&mounted, &desired));
    }
}
