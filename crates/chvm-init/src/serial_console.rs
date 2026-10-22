//! Serial rescue console.
//!
//! The kernel is booted with `console=ttyS0 console=hvc0` and the
//! hypervisor exposes ttyS0 through a PTY that `chvm console` attaches to
//! from the host. To make that port useful even when the main command,
//! networking, or the vsock servers are broken, we spawn a `/bin/sh`
//! session leader with its stdio wired to `/dev/ttyS0`. This is a pure
//! best-effort rescue path: every failure is a silent warning and never
//! blocks boot.

use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

const SERIAL_DEVICE: &str = "/dev/ttyS0";
const RESCUE_SHELL: &str = "/bin/sh";
const RESPAWN_DELAY: Duration = Duration::from_secs(1);

/// Open the serial port and keep a rescue shell running on it.
///
/// Runs on a background thread for the lifetime of the init process. If the
/// shell exits (e.g. the user types `exit` on the console), it is respawned
/// after a short delay so a later `chvm console` always finds a shell. The
/// PID-1 reaper collects the children this thread spawns.
pub fn run_rescue_shell() {
    // The serial device only exists when the kernel has the 8250 driver;
    // without it there is simply no rescue console.
    let tty = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(SERIAL_DEVICE)
    {
        Ok(tty) => tty,
        Err(e) => {
            eprintln!(
                "[init] Warning: cannot open {SERIAL_DEVICE}, serial rescue console disabled: {e}"
            );
            return;
        }
    };

    loop {
        match spawn_shell(&tty) {
            Ok(status) => {
                if let Some(code) = status.code() {
                    eprintln!("[init] Serial rescue shell exited ({code}), respawning");
                } else {
                    // Killed by a signal (e.g. shutdown): do not respawn, the
                    // init is going down and a fresh shell would fight it.
                    eprintln!("[init] Serial rescue shell terminated by signal");
                    return;
                }
            }
            Err(e) => {
                // Covers both spawn failures and a lost reap race with the
                // PID-1 reaper (ECHILD): the shell ended either way, so try
                // to give the next console connection a fresh one.
                eprintln!("[init] Warning: serial rescue shell failed: {e}");
            }
        }
        std::thread::sleep(RESPAWN_DELAY);
    }
}

/// Spawn `/bin/sh` as a session leader with stdio on the serial port.
fn spawn_shell(tty: &std::fs::File) -> std::io::Result<std::process::ExitStatus> {
    let stdin = tty.try_clone()?;
    let stdout = tty.try_clone()?;
    let stderr = tty.try_clone()?;

    let mut cmd = Command::new(RESCUE_SHELL);
    cmd.stdin(Stdio::from(stdin))
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    // Detach from init's session so the shell can own the serial line.
    // SAFETY: pre_exec runs in the forked child before exec; the closure
    // only calls async-signal-safe setsid and does not touch allocations.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn()?;
    child.wait()
}
