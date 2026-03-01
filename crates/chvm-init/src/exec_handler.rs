use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::FromRawFd;
use std::process::Command;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

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

// NOTE: Keep in sync with ExecResponse in chvm-lib/src/vsock/protocol.rs
#[derive(Serialize)]
struct ExecResponse {
    exit_code: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub fn handle_exec(fd: i32) {
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let mut reader = BufReader::new(&file);
    let mut writer = &file;

    // Read request
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        return;
    }

    let req: ExecRequest = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            let resp = ExecResponse {
                exit_code: -1,
                error: Some(format!("invalid request: {e}")),
            };
            send_response(&mut writer, &resp);
            return;
        }
    };

    if req.cmd.is_empty() {
        let resp = ExecResponse {
            exit_code: -1,
            error: Some("no command specified".into()),
        };
        send_response(&mut writer, &resp);
        return;
    }

    // Execute command
    let cmd_path = &req.cmd[0];
    let resolved = if !cmd_path.starts_with('/') {
        crate::helpers::find_binary(cmd_path, &[]).unwrap_or_else(|| cmd_path.clone())
    } else {
        cmd_path.clone()
    };

    let mut cmd = Command::new(&resolved);
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

    // Capture combined output
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let writer = Arc::new(Mutex::new(file));

    match cmd.spawn() {
        Ok(mut child) => {
            // Read stdout and stderr concurrently to avoid deadlock:
            // If child fills the stderr pipe buffer before stdout is drained, both block.
            // Both threads share the same vsock fd via Arc<Mutex<>> to prevent interleaved writes.
            let stderr = child.stderr.take();
            let stderr_thread = stderr.map(|stderr| {
                let w = Arc::clone(&writer);
                std::thread::spawn(move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines() {
                        match line {
                            Ok(l) => {
                                let formatted = format!("OUTPUT:{}\n", l);
                                let mut w = w.lock().unwrap_or_else(|e| e.into_inner());
                                if w.write_all(formatted.as_bytes()).is_err() || w.flush().is_err()
                                {
                                    eprintln!(
                                        "[exec] Warning: Failed to write stderr output over vsock"
                                    );
                                    break;
                                }
                            }
                            Err(_) => break,
                        }
                    }
                })
            });

            // Stream stdout directly to the client
            if let Some(stdout) = child.stdout.take() {
                let stdout_reader = BufReader::new(stdout);
                for line in stdout_reader.lines() {
                    match line {
                        Ok(l) => {
                            let formatted = format!("OUTPUT:{}\n", l);
                            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
                            if w.write_all(formatted.as_bytes()).is_err() || w.flush().is_err() {
                                eprintln!(
                                    "[exec] Warning: Failed to write stdout output over vsock"
                                );
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            }

            // Wait for stderr thread to finish before sending the response
            if let Some(handle) = stderr_thread {
                if handle.join().is_err() {
                    eprintln!("chvm-init: stderr relay thread panicked");
                }
            }

            let status = child.wait().ok();
            let exit_code = status.and_then(|s| s.code()).unwrap_or(1);
            let resp = ExecResponse {
                exit_code,
                error: None,
            };
            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
            send_response(&mut *w, &resp);
        }
        Err(e) => {
            let resp = ExecResponse {
                exit_code: -1,
                error: Some(format!("failed to start: {e}")),
            };
            let mut w = writer.lock().unwrap_or_else(|e| e.into_inner());
            send_response(&mut *w, &resp);
        }
    }
}

fn send_response(writer: &mut impl Write, resp: &ExecResponse) {
    if serde_json::to_writer(&mut *writer, resp).is_err()
        || writer.write_all(b"\n").is_err()
        || writer.flush().is_err()
    {
        eprintln!("[exec] Warning: Failed to send response over vsock");
    }
}
