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

/// Response from executing a command.
/// NOTE: Keep in sync with ExecResponse in chvm-init/src/exec_handler.rs
#[derive(Debug, Serialize, Deserialize)]
pub struct ExecResponse {
    pub exit_code: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

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
