//! Interactive serial console client (rescue path).
//!
//! cloud-hypervisor is launched with `--serial pty`; it allocates a PTY and
//! reports the slave path through the API socket (`vm.info` →
//! `config.serial.file`). The guest kernel is booted with
//! `console=ttyS0 console=hvc0`, so kernel messages and early panics are
//! visible on the serial port even when guest init or the vsock path is
//! broken, and chvm-init spawns a rescue `/bin/sh` on `/dev/ttyS0`.
//!
//! This module is the client side of that path:
//!
//! - [`serial_pty_path`] asks a running VMM for its serial PTY path.
//! - [`console`] splices the local terminal to the PTY: stdin goes to the
//!   serial port, serial output goes to stdout, Ctrl-Q exits, and the
//!   terminal is restored via a drop guard.
//!
//! The raw-mode guard and splice loop are ported from
//! `vsock/proxy.rs::interactive_session` (the `chvm attach` path) — kept
//! separate so the two paths can evolve independently.

use std::path::{Path, PathBuf};

use tokio::fs::OpenOptions;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::debug;

use crate::error::{ChvmError, Result};

/// Byte that detaches from the serial console (Ctrl-Q, minicom-style).
const EXIT_BYTE: u8 = 0x11;

/// RAII guard that puts the local terminal into raw mode and restores the
/// original settings on drop. Returns `None` when stdin is not a TTY.
struct RawModeGuard {
    original: libc::termios,
}

impl RawModeGuard {
    fn enable() -> Option<Self> {
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                return None;
            }
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return None;
            }
            Some(Self { original })
        }
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
    }
}

/// Query a running VMM for the PTY path of its serial console.
///
/// `api_socket` is the VM's cloud-hypervisor API socket (e.g.
/// `<state_dir>/api.sock`). Fails with a clear error when the VM is not
/// running (socket unreachable) or the VMM was not launched with
/// `--serial pty`.
pub async fn serial_pty_path(api_socket: &Path) -> Result<PathBuf> {
    let socket = api_socket.to_str().ok_or_else(|| {
        ChvmError::ApiError(format!(
            "API socket path is not valid UTF-8: {}",
            api_socket.display()
        ))
    })?;
    crate::hypervisor::api::serial_pty_path(socket).await
}

/// Attach the local terminal to the serial console PTY at `pty_path`.
///
/// Runs until the user presses Ctrl-Q, stdin/stdout closes, or the PTY
/// returns EIO (which is what a serial port does when the VM has shut down
/// and cloud-hypervisor closed the PTY master). The terminal is always
/// restored to its original mode, including on panic-free error returns.
pub async fn console(pty_path: &Path) -> Result<()> {
    let pty = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pty_path)
        .await
        .map_err(|e| {
            ChvmError::Io(std::io::Error::new(
                e.kind(),
                format!(
                    "failed to open serial console PTY {}: {e}",
                    pty_path.display()
                ),
            ))
        })?;

    eprintln!("Connected to serial console. Press Ctrl-Q to exit.");

    // Set terminal to raw mode so keystrokes are passed through unprocessed.
    // The guard restores the original settings when the splice ends.
    let _raw_guard = RawModeGuard::enable();

    let (mut pty_read, mut pty_write) = tokio::io::split(pty);

    // PTY -> stdout: ends on EOF or EIO (VM shut down).
    let pty_to_stdout = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            match pty_read.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut stdout = tokio::io::stdout();
                    if stdout.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
                Err(e) if e.raw_os_error() == Some(libc::EIO) => {
                    eprintln!("\r\n[serial console closed: VM shut down]");
                    break;
                }
                Err(e) => {
                    debug!(error = %e, "serial console read failed");
                    break;
                }
            }
        }
    });

    // stdin -> PTY: ends on EOF or Ctrl-Q.
    let stdin_to_pty = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        let mut stdin = tokio::io::stdin();
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = &buf[..n];
                    let exit_at = chunk.iter().position(|b| *b == EXIT_BYTE);
                    let end = exit_at.unwrap_or(chunk.len());
                    if pty_write.write_all(&chunk[..end]).await.is_err() {
                        break;
                    }
                    if exit_at.is_some() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    let _ = tokio::select! {
        r = pty_to_stdout => r,
        r = stdin_to_pty => r,
    };

    // _raw_guard is dropped here, restoring terminal settings.
    eprintln!("Disconnected from serial console.");
    Ok(())
}
