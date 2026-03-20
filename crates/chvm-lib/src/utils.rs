use std::collections::HashSet;
use std::fs::File;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use nix::fcntl::{Flock, FlockArg};
use tracing::debug;

/// A file lock guard that releases the lock when dropped.
/// Uses flock() for advisory locking.
pub struct FileLock {
    _flock: Flock<File>,
    path: PathBuf,
}

impl FileLock {
    /// Acquire an exclusive lock on the given path.
    /// Creates the lock file if it doesn't exist.
    /// Blocks until the lock is acquired.
    pub fn exclusive(path: &Path) -> std::io::Result<Self> {
        let lock_path = path.with_extension("lock");
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        let flock =
            Flock::lock(file, FlockArg::LockExclusive).map_err(|e| std::io::Error::other(e.1))?;

        Ok(Self {
            _flock: flock,
            path: lock_path,
        })
    }

    /// Acquire a shared (read) lock on the given path.
    /// Creates the lock file if it doesn't exist.
    /// Blocks until the lock is acquired.
    pub fn shared(path: &Path) -> std::io::Result<Self> {
        let lock_path = path.with_extension("lock");
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)?;

        let flock =
            Flock::lock(file, FlockArg::LockShared).map_err(|e| std::io::Error::other(e.1))?;

        Ok(Self {
            _flock: flock,
            path: lock_path,
        })
    }

    /// Get the path of the lock file.
    #[allow(dead_code)]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Write data to a file atomically by writing to a temp file and renaming.
/// This prevents corruption if the process crashes mid-write.
pub fn atomic_write(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;

    let parent = path.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(data)?;
    temp.flush()?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// Run a command and return an error if it fails.
pub fn run_cmd(cmd: &str, args: &[&str]) -> anyhow::Result<()> {
    let output = Command::new(cmd).args(args).output()?;
    if !output.status.success() {
        anyhow::bail!(
            "{} {} failed: {}",
            cmd,
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

/// Run a command, logging a debug message on failure instead of returning an error.
/// Use for best-effort operations like cleanup where failure is tolerable.
pub fn run_best_effort(cmd: &str, args: &[&str], context: &str) {
    match Command::new(cmd).args(args).output() {
        Ok(output) if !output.status.success() => {
            debug!(
                context = context,
                stderr = %String::from_utf8_lossy(&output.stderr).trim(),
                "Best-effort command failed"
            );
        }
        Err(e) => {
            debug!(context = context, error = %e, "Best-effort command failed to execute");
        }
        _ => {}
    }
}

/// Find a command binary by name, checking `which` first then falling back to extra paths.
pub fn find_command(name: &str, extra_paths: &[&str]) -> Option<String> {
    if let Ok(path) = which::which(name) {
        return Some(path.to_string_lossy().to_string());
    }
    for p in extra_paths {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

/// Resolve the invoking user's home directory when running under sudo/doas.
/// Falls back to the current user's home via `dirs::home_dir()`.
pub fn invoking_user_home() -> Option<std::path::PathBuf> {
    // Try SUDO_USER / DOAS_USER first — look up home via passwd (safe, no shell)
    if let Ok(user) = std::env::var("SUDO_USER").or_else(|_| std::env::var("DOAS_USER")) {
        if let Ok(Some(pw)) = nix::unistd::User::from_name(&user) {
            return Some(pw.dir);
        }
        // Fallback: /home/<user>
        let fallback = std::path::PathBuf::from("/home").join(&user);
        if fallback.is_dir() {
            return Some(fallback);
        }
    }
    dirs::home_dir()
}

/// Discover host DNS servers that are reachable from the guest.
/// Prefers systemd-resolved's uplink resolv.conf over the local stub resolver.
pub fn discover_host_dns_servers() -> Vec<String> {
    for path in ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"] {
        let Ok(contents) = std::fs::read_to_string(path) else {
            continue;
        };

        let servers = parse_resolv_conf_nameservers(&contents);
        if !servers.is_empty() {
            return servers;
        }
    }

    Vec::new()
}

fn parse_resolv_conf_nameservers(contents: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut servers = Vec::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let mut parts = line.split_whitespace();
        if parts.next() != Some("nameserver") {
            continue;
        }

        let Some(server) = parts.next() else {
            continue;
        };

        if is_unusable_nameserver(server) || !seen.insert(server.to_string()) {
            continue;
        }

        servers.push(server.to_string());
    }

    servers
}

fn is_unusable_nameserver(server: &str) -> bool {
    match server.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Ok(IpAddr::V6(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_command_with_which() {
        // Test with a command that should exist on most systems
        let result = find_command("sh", &[]);
        assert!(result.is_some());
        let path = result.unwrap();
        assert!(path.contains("sh"));
    }

    #[test]
    fn test_find_command_nonexistent() {
        let result = find_command("nonexistent-command-xyz-123", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_command_with_extra_paths() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let test_bin = temp_dir.path().join("testcmd");
        std::fs::write(&test_bin, "#!/bin/sh\necho test").unwrap();

        let extra_paths = vec![test_bin.to_str().unwrap()];
        let result = find_command("nonexistent-xyz", &extra_paths);

        assert!(result.is_some());
        assert_eq!(result.unwrap(), test_bin.to_string_lossy().to_string());
    }

    #[test]
    fn test_find_command_prefers_which() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let fake_sh = temp_dir.path().join("sh");
        std::fs::write(&fake_sh, "fake").unwrap();

        // Should find real 'sh' via which, not our fake one
        let extra_paths = vec![fake_sh.to_str().unwrap()];
        let result = find_command("sh", &extra_paths);

        assert!(result.is_some());
        // Real sh should not be in our temp directory
        assert!(!result.unwrap().contains(temp_dir.path().to_str().unwrap()));
    }

    #[test]
    fn test_run_cmd_success() {
        let result = run_cmd("echo", &["test"]);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_cmd_failure() {
        let result = run_cmd("false", &[]);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("false"));
    }

    #[test]
    fn test_run_cmd_nonexistent() {
        let result = run_cmd("nonexistent-command-xyz", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_best_effort_success() {
        // Should not panic
        run_best_effort("echo", &["test"], "test context");
    }

    #[test]
    fn test_run_best_effort_failure() {
        // Should not panic even on failure
        run_best_effort("false", &[], "test failure");
    }

    #[test]
    fn test_run_best_effort_nonexistent() {
        // Should not panic even if command doesn't exist
        run_best_effort("nonexistent-xyz-123", &[], "test nonexistent");
    }

    #[test]
    fn test_invoking_user_home_returns_some() {
        let home = invoking_user_home();
        assert!(home.is_some());
    }

    #[test]
    fn test_invoking_user_home_with_sudo_user() {
        // Save original value
        let original = std::env::var("SUDO_USER").ok();

        // Set SUDO_USER to current user (should exist)
        if let Ok(current_user) = std::env::var("USER") {
            std::env::set_var("SUDO_USER", &current_user);
            let home = invoking_user_home();
            assert!(home.is_some());

            // Cleanup
            if let Some(orig) = original {
                std::env::set_var("SUDO_USER", orig);
            } else {
                std::env::remove_var("SUDO_USER");
            }
        }
    }

    #[test]
    fn test_parse_resolv_conf_nameservers_prefers_real_servers() {
        let contents = r#"
        # comment
        nameserver 127.0.0.53
        nameserver 192.168.1.1
        nameserver 1.1.1.1
        nameserver 192.168.1.1
        "#;

        let servers = parse_resolv_conf_nameservers(contents);

        assert_eq!(servers, vec!["192.168.1.1", "1.1.1.1"]);
    }

    #[test]
    fn test_parse_resolv_conf_nameservers_keeps_ipv6() {
        let contents = r#"
        nameserver ::1
        nameserver 2001:4860:4860::8888
        "#;

        let servers = parse_resolv_conf_nameservers(contents);

        assert_eq!(servers, vec!["2001:4860:4860::8888"]);
    }

    #[test]
    fn test_file_lock_exclusive() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.json");
        std::fs::write(&test_file, "{}").unwrap();

        // Should be able to acquire exclusive lock
        let lock = FileLock::exclusive(&test_file);
        assert!(lock.is_ok());

        // Lock is released when dropped
        drop(lock);

        // Should be able to acquire again
        let lock2 = FileLock::exclusive(&test_file);
        assert!(lock2.is_ok());
    }

    #[test]
    fn test_file_lock_shared() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.json");
        std::fs::write(&test_file, "{}").unwrap();

        // Should be able to acquire multiple shared locks
        let lock1 = FileLock::shared(&test_file);
        assert!(lock1.is_ok());

        let lock2 = FileLock::shared(&test_file);
        assert!(lock2.is_ok());
    }

    #[test]
    fn test_atomic_write() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.json");

        let data = b"{\"key\": \"value\"}";
        atomic_write(&test_file, data).unwrap();

        let contents = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(contents, "{\"key\": \"value\"}");
    }

    #[test]
    fn test_atomic_write_overwrites() {
        use tempfile::TempDir;

        let temp_dir = TempDir::new().unwrap();
        let test_file = temp_dir.path().join("test.json");

        std::fs::write(&test_file, "old content").unwrap();

        let data = b"new content";
        atomic_write(&test_file, data).unwrap();

        let contents = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(contents, "new content");
    }
}
