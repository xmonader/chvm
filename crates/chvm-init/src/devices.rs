use std::os::unix::fs::PermissionsExt;

pub fn create_devices() -> Result<(), Box<dyn std::error::Error>> {
    let devices: &[(&str, u32, u32, u64, u64)] = &[
        ("/dev/null", libc::S_IFCHR | 0o666, 0, 1, 3),
        ("/dev/zero", libc::S_IFCHR | 0o666, 0, 1, 5),
        ("/dev/random", libc::S_IFCHR | 0o666, 0, 1, 8),
        ("/dev/urandom", libc::S_IFCHR | 0o666, 0, 1, 9),
        ("/dev/console", libc::S_IFCHR | 0o600, 0, 5, 1),
        ("/dev/tty", libc::S_IFCHR | 0o666, 0, 5, 0),
        ("/dev/net/tun", libc::S_IFCHR | 0o666, 0, 10, 200),
    ];

    let _ = std::fs::create_dir_all("/dev/net");

    for &(path, mode, _extra, major, minor) in devices {
        let _ = std::fs::remove_file(path);
        let dev = nix::sys::stat::makedev(major, minor);
        unsafe {
            let c_path = std::ffi::CString::new(path)?;
            let ret = libc::mknod(c_path.as_ptr(), mode, dev as libc::dev_t);
            if ret < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() != Some(libc::EEXIST) {
                    eprintln!("[init] Warning: Failed to create {path}: {err}");
                    continue;
                }
            }
        }

        if let Err(err) =
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode & 0o7777))
        {
            eprintln!("[init] Warning: Failed to chmod {path}: {err}");
        }
    }

    Ok(())
}
