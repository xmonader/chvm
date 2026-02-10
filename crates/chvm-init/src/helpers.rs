/// Standard PATH directories for binary lookup.
pub const STANDARD_PATH: &[&str] = &[
    "/usr/local/sbin", "/usr/local/bin",
    "/usr/sbin", "/usr/bin",
    "/sbin", "/bin",
];

/// Standard PATH string for environment setup.
pub const PATH_ENV: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Default DNS servers.
pub const DNS_SERVERS: &[&str] = &["8.8.8.8", "8.8.4.4", "1.1.1.1"];

/// Kernel modules needed for vsock and virtiofs (in dependency order).
/// NOTE: Keep in sync with KERNEL_MODULES in chvm-lib/src/storage/image.rs
/// (the host side uses full relative paths for injection from the host kernel tree).
pub const GUEST_KERNEL_MODULES: &[&str] = &[
    "vsock.ko",
    "vmw_vsock_virtio_transport_common.ko",
    "vmw_vsock_virtio_transport.ko",
    "virtiofs.ko",
];

/// Find a binary by name, searching the given directories.
/// Returns the full path if found.
pub fn find_binary(name: &str, extra_dirs: &[&str]) -> Option<String> {
    // Check extra dirs first (specific paths)
    for dir in extra_dirs {
        let path = if dir.contains('/') && !dir.ends_with('/') {
            // It's already a full path
            dir.to_string()
        } else {
            format!("{dir}/{name}")
        };
        if std::path::Path::new(&path).exists() {
            return Some(path);
        }
    }

    // Then check standard PATH directories
    for dir in STANDARD_PATH {
        let full = format!("{dir}/{name}");
        if std::path::Path::new(&full).exists() {
            return Some(full);
        }
    }
    None
}

/// Set standard environment variables (PATH, HOME, TERM) on a Command.
/// HOME is /root because chvm-init runs as PID 1 (root) and doesn't support
/// user switching yet. If user support is added, this should resolve from /etc/passwd.
pub fn setup_standard_env(cmd: &mut std::process::Command) {
    cmd.env("PATH", PATH_ENV);
    cmd.env("HOME", "/root");
    cmd.env("TERM", "xterm");
}
