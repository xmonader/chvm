use std::process::Command;

use tracing::debug;

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
}
