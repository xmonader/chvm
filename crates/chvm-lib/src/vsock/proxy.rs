use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::unix::OwnedWriteHalf;
use tokio::net::UnixStream;
use tokio::sync::Mutex;

use crate::error::{ChvmError, Result};
use crate::vm::manager::ExecOptions;
use crate::vsock::protocol::*;

const VSOCK_HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Connect to a VM's vsock via the CH UDS proxy.
/// Sends "CONNECT <port>\n", expects "OK\n", then returns the stream.
async fn connect_vsock(socket_path: &str, port: u32) -> Result<UnixStream> {
    let mut stream =
        tokio::time::timeout(VSOCK_HANDSHAKE_TIMEOUT, UnixStream::connect(socket_path))
            .await
            .map_err(|_| {
                ChvmError::VsockError(format!(
                    "timed out connecting to vsock proxy {}",
                    socket_path
                ))
            })?
            .map_err(|e| {
                ChvmError::VsockError(format!("failed to connect to {}: {e}", socket_path))
            })?;

    // Send CONNECT command
    let connect_cmd = format!("CONNECT {port}\n");
    tokio::time::timeout(
        VSOCK_HANDSHAKE_TIMEOUT,
        stream.write_all(connect_cmd.as_bytes()),
    )
    .await
    .map_err(|_| ChvmError::VsockError("timed out sending CONNECT to vsock proxy".into()))?
    .map_err(|e| ChvmError::VsockError(format!("failed to send CONNECT: {e}")))?;

    // Read OK response
    let mut buf = vec![0u8; 256];
    let n = tokio::time::timeout(VSOCK_HANDSHAKE_TIMEOUT, stream.read(&mut buf))
        .await
        .map_err(|_| {
            ChvmError::VsockError(format!(
                "timed out waiting for CONNECT response on {}",
                socket_path
            ))
        })?
        .map_err(|e| ChvmError::VsockError(format!("failed to read CONNECT response: {e}")))?;

    let resp = String::from_utf8_lossy(&buf[..n]);
    if !resp.starts_with("OK") {
        return Err(ChvmError::VsockError(format!(
            "vsock CONNECT failed: {}",
            resp.trim()
        )));
    }

    Ok(stream)
}

/// Whether to print output to stdout or capture it into a String.
enum OutputMode {
    Print,
    Capture,
}

/// Shared exec implementation. Sends an ExecRequest over vsock and processes the response.
async fn exec_inner(
    vsock_path: &str,
    opts: &ExecOptions,
    mode: OutputMode,
) -> Result<(i32, String)> {
    let mut stream = connect_vsock(vsock_path, VSOCK_PORT_EXEC).await?;

    let req = ExecRequest {
        cmd: opts.cmd.clone(),
        env: opts.env.clone(),
        working_dir: opts.working_dir.clone(),
    };

    let mut req_bytes = serde_json::to_vec(&req)?;
    req_bytes.push(b'\n');
    stream
        .write_all(&req_bytes)
        .await
        .map_err(|e| ChvmError::VsockError(format!("failed to send request: {e}")))?;

    let mut reader = tokio::io::BufReader::new(stream);
    let mut captured = String::new();
    let mut stdout = tokio::io::stdout();
    let mut stderr = tokio::io::stderr();
    let mut header = [0u8; FRAME_HEADER_LEN];

    loop {
        // Read the fixed-size frame header. Clean EOF here means the connection
        // closed before an EXIT frame — treat as a crashed guest.
        if reader.read_exact(&mut header).await.is_err() {
            let _ = stdout.flush().await;
            return Err(ChvmError::VsockError(
                "connection closed without receiving exit status (guest may have crashed)".into(),
            ));
        }
        let frame_type = header[0];
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len > MAX_FRAME_PAYLOAD {
            return Err(ChvmError::VsockError(format!(
                "exec frame too large ({len} bytes)"
            )));
        }
        let mut payload = vec![0u8; len];
        reader
            .read_exact(&mut payload)
            .await
            .map_err(|e| ChvmError::VsockError(format!("truncated exec frame: {e}")))?;

        match frame_type {
            FRAME_STDOUT => match mode {
                OutputMode::Print => {
                    stdout.write_all(&payload).await.ok();
                    stdout.flush().await.ok();
                }
                OutputMode::Capture => captured.push_str(&String::from_utf8_lossy(&payload)),
            },
            FRAME_STDERR => match mode {
                OutputMode::Print => {
                    stderr.write_all(&payload).await.ok();
                    stderr.flush().await.ok();
                }
                OutputMode::Capture => captured.push_str(&String::from_utf8_lossy(&payload)),
            },
            FRAME_ERROR => {
                let msg = String::from_utf8_lossy(&payload);
                if let OutputMode::Print = mode {
                    eprintln!("{msg}");
                }
            }
            FRAME_EXIT => {
                let code = if payload.len() == 4 {
                    i32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]])
                } else {
                    1
                };
                let _ = stdout.flush().await;
                let _ = stderr.flush().await;
                return Ok((code, captured));
            }
            _ => { /* unknown frame type — ignore for forward compat */ }
        }
    }
}

/// Execute a command in a running VM and print output.
pub async fn exec_command(vsock_path: &str, opts: &ExecOptions) -> Result<i32> {
    let (code, _) = exec_inner(vsock_path, opts, OutputMode::Print).await?;
    Ok(code)
}

/// Execute a command in a running VM and capture output as a string.
/// Returns (exit_code, captured_output).
pub async fn exec_capture(vsock_path: &str, opts: &ExecOptions) -> Result<(i32, String)> {
    exec_inner(vsock_path, opts, OutputMode::Capture).await
}

/// RAII guard that restores terminal settings on drop.
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

fn terminal_size() -> Option<(u16, u16)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut ws) != 0 {
            return None;
        }
        if ws.ws_row == 0 || ws.ws_col == 0 {
            return None;
        }
        Some((ws.ws_row, ws.ws_col))
    }
}

async fn send_resize(writer: &Arc<Mutex<OwnedWriteHalf>>) -> std::io::Result<()> {
    let Some((rows, cols)) = terminal_size() else {
        return Ok(());
    };
    let msg = encode_resize(rows, cols);
    let mut writer = writer.lock().await;
    writer.write_all(&msg).await
}

/// Open an interactive shell session to a running VM.
/// Detach with Ctrl+P, Ctrl+Q (same as Docker). Use a regular terminal, not IDE terminal.
pub async fn interactive_session(vsock_path: &str) -> Result<i32> {
    let stream = connect_vsock(vsock_path, VSOCK_PORT_SHELL).await?;

    eprintln!("Attached. Detach with Ctrl+P, Ctrl+Q (use regular terminal, not IDE).");

    // Set terminal to raw mode so keystrokes are passed through unprocessed
    let _raw_guard = RawModeGuard::enable();

    // Bidirectional proxy: stdin <-> vsock, vsock <-> stdout
    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer));

    let _ = send_resize(&writer).await;

    let stdout_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let mut stdout = tokio::io::stdout();
                    if stdout.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                    let _ = stdout.flush().await;
                }
                Err(_) => break,
            }
        }
    });

    let resize_writer = writer.clone();
    let resize_handle = tokio::spawn(async move {
        #[cfg(unix)]
        {
            if let Ok(mut sigwinch) =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
            {
                while sigwinch.recv().await.is_some() {
                    if send_resize(&resize_writer).await.is_err() {
                        break;
                    }
                }
            }
        }
    });

    let stdin_writer = writer.clone();
    let stdin_handle = tokio::spawn(async move {
        let mut buf = vec![0u8; 4096];
        let mut stdin = tokio::io::stdin();
        let mut saw_ctrl_p = false;
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    // Check for Ctrl+P (0x10), Ctrl+Q (0x11) escape sequence
                    let mut i = 0;
                    while i < n {
                        if saw_ctrl_p {
                            if buf[i] == 0x11 {
                                // Ctrl+P, Ctrl+Q — detach
                                return;
                            }
                            // False alarm: send the buffered Ctrl+P, then this byte
                            let _ = stdin_writer.lock().await.write_all(&[0x10]).await;
                            saw_ctrl_p = false;
                        }

                        if buf[i] == 0x10 {
                            // Ctrl+P — buffer it, wait for next byte
                            saw_ctrl_p = true;
                            i += 1;
                            continue;
                        }

                        // Find the next Ctrl+P in the remaining buffer, or send everything
                        let start = i;
                        while i < n && buf[i] != 0x10 {
                            i += 1;
                        }
                        if stdin_writer
                            .lock()
                            .await
                            .write_all(&buf[start..i])
                            .await
                            .is_err()
                        {
                            return;
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    let _ = tokio::select! {
        r = stdout_handle => r,
        r = stdin_handle => r,
    };

    resize_handle.abort();

    // _raw_guard is dropped here, restoring terminal settings
    Ok(0)
}
