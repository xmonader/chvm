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
