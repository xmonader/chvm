use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::Request;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tracing::debug;

use crate::error::{ChvmError, Result};

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
