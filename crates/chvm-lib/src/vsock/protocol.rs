use serde::{Deserialize, Serialize};

/// Vsock port for exec command requests.
pub const VSOCK_PORT_EXEC: u32 = 1024;

/// Vsock port for interactive shell sessions.
pub const VSOCK_PORT_SHELL: u32 = 1025;

/// CID that accepts connections from any source.
pub const VSOCK_CID_ANY: u32 = 0xFFFFFFFF;

/// Request to execute a command in the guest.
/// NOTE: Keep in sync with ExecRequest in chvm-init/src/exec_handler.rs
/// (chvm-init is a separate guest binary and can't depend on chvm-lib).
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecRequest {
    pub cmd: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub working_dir: Option<String>,
}

/// Exec response framing (guest -> host).
///
/// Each frame is `[1 byte type][4 bytes big-endian u32 length][payload]`. This
/// replaces the old line-based `OUTPUT:` / trailing-JSON protocol, which could
/// not carry binary output (UTF-8 line splitting) and let output that happened
/// to parse as JSON spoof the exit status. Now stdout/stderr bytes are relayed
/// verbatim and the exit code is a dedicated typed frame.
///
/// NOTE: Keep these constants in sync with chvm-init/src/exec_handler.rs
/// (chvm-init is a separate guest binary and can't depend on chvm-lib).
pub const FRAME_STDOUT: u8 = 1;
pub const FRAME_STDERR: u8 = 2;
/// Terminal frame; payload is a 4-byte big-endian i32 exit code.
pub const FRAME_EXIT: u8 = 3;
/// Out-of-band error (e.g. failed to spawn); payload is a UTF-8 message.
pub const FRAME_ERROR: u8 = 4;
pub const FRAME_HEADER_LEN: usize = 5;
/// Largest single-frame payload the host will accept, to bound allocation.
pub const MAX_FRAME_PAYLOAD: usize = 16 * 1024 * 1024;

/// In-band resize escape: \x01RSZ (SOH + "RSZ").
/// NOTE: Keep in sync with RESIZE_MAGIC in chvm-init/src/shell_handler.rs
pub const RESIZE_MAGIC: [u8; 4] = [0x01, 0x52, 0x53, 0x5a];
pub const RESIZE_MSG_LEN: usize = 8;

pub fn encode_resize(rows: u16, cols: u16) -> [u8; RESIZE_MSG_LEN] {
    let mut msg = [0u8; RESIZE_MSG_LEN];
    msg[..4].copy_from_slice(&RESIZE_MAGIC);
    msg[4..6].copy_from_slice(&rows.to_be_bytes());
    msg[6..8].copy_from_slice(&cols.to_be_bytes());
    msg
}
