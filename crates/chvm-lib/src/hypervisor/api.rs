use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::rt::TokioIo;
use std::path::PathBuf;
use tokio::net::UnixStream;
use tracing::debug;

use crate::error::{ChvmError, Result};

/// Serial console configuration as reported by `vm.info` under `config.serial`
/// (cloud-hypervisor's `ConsoleConfig`).
///
/// With `--serial pty`, cloud-hypervisor allocates a PTY at VM start and
/// reports the slave path in `file` (mode `Pty`). The `path` field is also
/// accepted so the parser keeps working if a future CH version renames the
/// field.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SerialConsoleInfo {
    /// PTY path for `mode: "Pty"`.
    #[serde(default)]
    pub file: Option<PathBuf>,
    /// Tolerated alias for `file`.
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Console output mode, e.g. "Pty", "Off", "File", "Tty".
    #[serde(default)]
    pub mode: Option<String>,
}

impl SerialConsoleInfo {
    /// The PTY path, if this serial console is a PTY and the VMM reported one.
    fn pty_path(&self) -> Option<PathBuf> {
        self.file.clone().or_else(|| self.path.clone())
    }
}

/// Subset of the `vm.info` `config` object we care about. Unknown fields
/// (cpus, memory, net, ...) are ignored.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VmConfigInfo {
    #[serde(default)]
    pub serial: Option<SerialConsoleInfo>,
}

/// Typed `GET /vm.info` response (only the fields chvm uses).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VmInfo {
    #[serde(default)]
    pub config: Option<VmConfigInfo>,
}

impl VmInfo {
    /// Extract the serial PTY path for a VMM launched with `--serial pty`.
    ///
    /// Returns a clear error when the VM was not launched with a PTY serial
    /// console: the serial config is absent (cloud-hypervisor too old to
    /// report it, or not configured) or has no allocated PTY path yet.
    pub fn serial_pty_path(&self) -> Result<PathBuf> {
        let serial = self.config.as_ref().and_then(|c| c.serial.as_ref());
        let Some(serial) = serial else {
            return Err(ChvmError::ApiError(
                "VM has no serial console configured (cloud-hypervisor too \
                 old, or not launched with --serial pty)"
                    .into(),
            ));
        };
        serial.pty_path().ok_or_else(|| {
            let mode = serial.mode.as_deref().unwrap_or("unknown");
            ChvmError::ApiError(format!(
                "serial console has no PTY path (mode={mode}); is this VM \
                 running a cloud-hypervisor with --serial pty?"
            ))
        })
    }
}

/// Make an HTTP request to the Cloud Hypervisor API via Unix socket.
async fn api_request(
    socket_path: &str,
    method: &str,
    path: &str,
    body: Option<serde_json::Value>,
) -> Result<Vec<u8>> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| ChvmError::ApiError(format!("failed to connect to API socket: {e}")))?;

    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| ChvmError::ApiError(format!("handshake failed: {e}")))?;

    tokio::spawn(async move {
        if let Err(e) = conn.await {
            debug!(error = %e, "API connection error");
        }
    });

    let body_bytes = match body {
        Some(val) => Full::new(Bytes::from(serde_json::to_vec(&val)?)),
        None => Full::new(Bytes::new()),
    };

    let req = Request::builder()
        .method(method)
        .uri(format!("http://localhost{path}"))
        .header("Content-Type", "application/json")
        .body(body_bytes)
        .map_err(|e| ChvmError::ApiError(format!("failed to build request: {e}")))?;

    let resp = sender
        .send_request(req)
        .await
        .map_err(|e| ChvmError::ApiError(format!("request failed: {e}")))?;

    let status = resp.status();
    let body = resp
        .into_body()
        .collect()
        .await
        .map_err(|e| ChvmError::ApiError(format!("failed to read response: {e}")))?
        .to_bytes()
        .to_vec();

    if status.as_u16() >= 400 {
        let msg = String::from_utf8_lossy(&body);
        return Err(ChvmError::ApiError(format!("API error {}: {msg}", status)));
    }

    Ok(body)
}

/// Ping the VMM to check it's alive.
pub async fn ping(socket_path: &str) -> Result<()> {
    api_request(socket_path, "GET", "/api/v1/vmm.ping", None).await?;
    Ok(())
}

/// Get VM info.
pub async fn vm_info(socket_path: &str) -> Result<serde_json::Value> {
    let body = api_request(socket_path, "GET", "/api/v1/vm.info", None).await?;
    let info: serde_json::Value = serde_json::from_slice(&body)?;
    Ok(info)
}

/// Get VM info as a typed response.
pub async fn vm_info_typed(socket_path: &str) -> Result<VmInfo> {
    let body = api_request(socket_path, "GET", "/api/v1/vm.info", None).await?;
    let info: VmInfo = serde_json::from_slice(&body)?;
    Ok(info)
}

/// Query the running VMM for the PTY path of its serial console.
///
/// Works for any VMM launched with `--serial pty`; the PTY is allocated at
/// VM start and reported via `vm.info` (`config.serial.file`). Fails with a
/// clear error when the socket is unreachable (VM not running) or the VMM
/// has no PTY serial console.
pub async fn serial_pty_path(socket_path: &str) -> Result<PathBuf> {
    let info = vm_info_typed(socket_path).await?;
    info.serial_pty_path()
}

/// Get VM counters (network and block device statistics).
pub async fn vm_counters(socket_path: &str) -> Result<serde_json::Value> {
    let body = api_request(socket_path, "GET", "/api/v1/vm.counters", None).await?;
    let counters: serde_json::Value = serde_json::from_slice(&body)?;
    Ok(counters)
}

/// Gracefully shutdown the VM (powers off the guest but leaves the VMM process
/// running so it can be rebooted). Use `vmm_shutdown` to also exit the process.
pub async fn shutdown(socket_path: &str) -> Result<()> {
    api_request(socket_path, "PUT", "/api/v1/vm.shutdown", None).await?;
    Ok(())
}

/// Shut down the whole cloud-hypervisor VMM, terminating the process. This is
/// what `stop` wants: it powers off the guest and makes the process exit
/// promptly instead of lingering until SIGKILL.
pub async fn vmm_shutdown(socket_path: &str) -> Result<()> {
    api_request(socket_path, "PUT", "/api/v1/vmm.shutdown", None).await?;
    Ok(())
}

/// Power button (ACPI).
pub async fn power_button(socket_path: &str) -> Result<()> {
    api_request(socket_path, "PUT", "/api/v1/vm.power-button", None).await?;
    Ok(())
}

/// Reboot the VM.
pub async fn reboot(socket_path: &str) -> Result<()> {
    api_request(socket_path, "PUT", "/api/v1/vm.reboot", None).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sample `vm.info` payload shaped like cloud-hypervisor v54 responds
    /// for a VM launched with `--serial pty --console file=...`.
    const VM_INFO_SERIAL_PTY: &str = r#"{
        "config": {
            "console": {
                "file": "/var/lib/chvm/vms/abc123/console.log",
                "mode": "File",
                "iommu": false,
                "socket": null
            },
            "serial": {
                "file": "/dev/pts/12",
                "mode": "Pty",
                "iommu": false,
                "socket": null
            },
            "cpus": { "boot_vcpus": 1, "max_vcpus": 1 },
            "memory": { "size": 536870912 }
        },
        "state": "Running",
        "memory_actual_size": 536870912,
        "device_tree": null
    }"#;

    #[test]
    fn vm_info_parses_serial_pty_path() {
        let info: VmInfo = serde_json::from_str(VM_INFO_SERIAL_PTY).unwrap();
        let path = info.serial_pty_path().unwrap();
        assert_eq!(path, PathBuf::from("/dev/pts/12"));
    }

    #[test]
    fn vm_info_serial_pty_path_accepts_path_alias() {
        // Tolerate a future CH that renames the field to `path`.
        let payload = r#"{
            "config": { "serial": { "path": "/dev/pts/7", "mode": "Pty" } },
            "state": "Running"
        }"#;
        let info: VmInfo = serde_json::from_str(payload).unwrap();
        assert_eq!(info.serial_pty_path().unwrap(), PathBuf::from("/dev/pts/7"));
    }

    #[test]
    fn vm_info_serial_off_has_no_pty_path() {
        let payload = r#"{
            "config": { "serial": { "file": null, "mode": "Off", "socket": null } },
            "state": "Running"
        }"#;
        let info: VmInfo = serde_json::from_str(payload).unwrap();
        let err = info.serial_pty_path().unwrap_err();
        assert!(err.to_string().contains("mode=Off"), "unexpected: {err}");
    }

    #[test]
    fn vm_info_missing_serial_is_clear_error() {
        let payload = r#"{ "config": { "cpus": { "boot_vcpus": 1 } }, "state": "Running" }"#;
        let info: VmInfo = serde_json::from_str(payload).unwrap();
        let err = info.serial_pty_path().unwrap_err();
        assert!(
            err.to_string().contains("no serial console configured"),
            "unexpected: {err}"
        );
    }
}
