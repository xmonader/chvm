use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::io::FromRawFd;
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde::Deserialize;

// NOTE: Keep in sync with ExecRequest in chvm-lib/src/vsock/protocol.rs
// (chvm-init is a separate guest binary and can't depend on chvm-lib).
#[derive(Deserialize)]
struct ExecRequest {
    cmd: Vec<String>,
    #[serde(default)]
    env: Vec<String>,
    #[serde(default)]
    working_dir: Option<String>,
}

// Exec response framing: [1 byte type][4 bytes big-endian u32 length][payload].
// NOTE: Keep in sync with the FRAME_* constants in
// chvm-lib/src/vsock/protocol.rs.
const FRAME_STDOUT: u8 = 1;
const FRAME_STDERR: u8 = 2;
const FRAME_EXIT: u8 = 3;
const FRAME_ERROR: u8 = 4;

fn send_frame(w: &mut impl Write, frame_type: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut header = [0u8; 5];
    header[0] = frame_type;
    header[1..5].copy_from_slice(&(payload.len() as u32).to_be_bytes());
    w.write_all(&header)?;
    w.write_all(payload)?;
    w.flush()
}

fn send_exit(w: &mut impl Write, code: i32) {
    let _ = send_frame(w, FRAME_EXIT, &code.to_be_bytes());
}

fn send_error(w: &mut impl Write, msg: &str) {
    let _ = send_frame(w, FRAME_ERROR, msg.as_bytes());
}

/// Relay raw bytes from a child stream to the client as framed output. Bytes are
/// forwarded verbatim (no line splitting) so binary output survives intact.
fn relay_stream(mut reader: impl Read, writer: &Arc<Mutex<File>>, frame_type: u8) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                if send_frame(&mut *w, frame_type, &buf[..n]).is_err() {
                    break;
                }
            }
        }
    }
}

pub fn handle_exec(fd: i32) {
    let file = unsafe { File::from_raw_fd(fd) };

    // Read request, bounded so a client that never sends '\n' can't OOM PID 1.
    const MAX_REQUEST_BYTES: u64 = 1 << 20; // 1 MiB
    let mut reader = BufReader::new((&file).take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }

    let mut early = &file;
    let req: ExecRequest = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            send_error(&mut early, &format!("invalid request: {e}"));
            send_exit(&mut early, -1);
            return;
        }
    };

    if req.cmd.is_empty() {
        send_error(&mut early, "no command specified");
        send_exit(&mut early, -1);
        return;
    }

    let cmd_path = &req.cmd[0];
    let mut cmd = Command::new(cmd_path);
    cmd.args(&req.cmd[1..]);

    for env_var in &req.env {
        if let Some((key, value)) = env_var.split_once('=') {
            cmd.env(key, value);
        }
    }
    crate::helpers::setup_standard_env(&mut cmd);

    if let Some(ref dir) = req.working_dir {
        cmd.current_dir(dir);
    }

    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let writer = Arc::new(Mutex::new(file));

    match cmd.spawn() {
        Ok(mut child) => {
            // PID 1's reaper owns waitpid(-1), so we can't reliably wait() on the
            // child ourselves — it would race the reaper and get ECHILD. Instead
            // read the real exit code the reaper publishes for this pid.
            let pid = child.id() as i32;

            // Drain stdout and stderr concurrently to avoid a pipe-buffer
            // deadlock; both share the vsock fd via the mutex so frames never
            // interleave.
            let stderr = child.stderr.take();
            let stderr_thread = stderr.map(|stderr| {
                let w = Arc::clone(&writer);
                std::thread::spawn(move || relay_stream(stderr, &w, FRAME_STDERR))
            });

            if let Some(stdout) = child.stdout.take() {
                relay_stream(stdout, &writer, FRAME_STDOUT);
            }

            if let Some(handle) = stderr_thread {
                if handle.join().is_err() {
                    eprintln!("chvm-init: stderr relay thread panicked");
                }
            }

            // Output pipes are closed, so the process has exited or is about to;
            // wait for the reaper to publish its exit code.
            let exit_code =
                crate::signal::wait_exec_exit(pid, std::time::Duration::from_secs(10)).unwrap_or(1);
            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
            send_exit(&mut *w, exit_code);
        }
        Err(e) => {
            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
            send_error(&mut *w, &format!("failed to start: {e}"));
            send_exit(&mut *w, -1);
        }
    }
}
