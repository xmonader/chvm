use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::net::UnixStream;
use tokio::process::Command;
use tracing::{debug, info};

use crate::error::{ChvmError, Result};
use crate::vm::state::{RootfsType, VmConfig};

/// Push a `--flag value` pair onto the args list.
fn push_arg(args: &mut Vec<String>, flag: &str, value: impl Into<String>) {
    args.push(flag.into());
    args.push(value.into());
}

/// Build the cloud-hypervisor command line arguments from VmConfig.
pub fn build_args(config: &VmConfig, vm_dir: &Path) -> Vec<String> {
    let socket_path = vm_dir.join("api.sock");
    let default_log = vm_dir.join("console.log");
    let default_log_str = default_log.to_string_lossy();
    let log_path = config.log_path.as_deref().unwrap_or(&default_log_str);
    let ch_log_path = vm_dir.join("cloud-hypervisor.log");

    let mut args = Vec::new();
    push_arg(&mut args, "--api-socket", socket_path.to_string_lossy());
    push_arg(&mut args, "--cpus", format!("boot={}", config.vcpu));
    push_arg(&mut args, "--memory", format!("size={}M,shared=on", config.memory_mb));
    push_arg(&mut args, "--kernel", &config.kernel.path);
    push_arg(&mut args, "--cmdline", &config.kernel.cmdline);

    if let Some(ref initrd) = config.kernel.initrd_path {
        push_arg(&mut args, "--initramfs", initrd);
    }

    match config.rootfs.rootfs_type {
        RootfsType::VirtioFs => push_arg(&mut args, "--fs", format!("tag=rootfs,socket={}", config.rootfs.source)),
        RootfsType::VirtioBlock => push_arg(&mut args, "--disk", format!("path={}", config.rootfs.source)),
    }

    push_arg(&mut args, "--console", format!("file={log_path}"));
    push_arg(&mut args, "--serial", "off");

    if let Some(ref tap) = config.network.tap_device {
        let mut net_arg = format!("tap={tap}");
        if let Some(ref mac) = config.network.mac_address {
            net_arg.push_str(&format!(",mac={mac}"));
        }
        push_arg(&mut args, "--net", net_arg);
    }

    for mount in &config.mounts {
        if mount.mount_type.as_deref() == Some("virtiofs") || mount.mount_type.is_none() {
            let tag = mount.tag.as_deref().unwrap_or(&mount.target);
            push_arg(&mut args, "--fs", format!("tag={tag},socket={}", mount.source));
        }
    }

    if config.vsock_cid > 0 {
        let vsock_sock = vm_dir.join("vsock.sock");
        push_arg(&mut args, "--vsock", format!("cid={},socket={}", config.vsock_cid, vsock_sock.display()));
    }

    push_arg(&mut args, "--log-file", ch_log_path.to_string_lossy());

    args
}

/// Launch a Cloud Hypervisor process and return its PID.
pub async fn launch(config: &VmConfig, vm_dir: &Path) -> Result<i32> {
    let ch_bin = find_ch_binary().ok_or_else(|| {
        ChvmError::HypervisorNotFound(
            "cloud-hypervisor not found. Install it or set CHVM_CH_BIN env var.".into()
        )
    })?;

    let args = build_args(config, vm_dir);

    debug!(bin = %ch_bin, args = ?args, "Launching cloud-hypervisor");

    let default_launch_log = vm_dir.join("console.log");
    let default_launch_log_str = default_launch_log.to_string_lossy();
    let launch_log_path = config.log_path.as_deref()
        .unwrap_or(&default_launch_log_str);
    let log_file = std::fs::File::create(launch_log_path)?;
    let log_file2 = log_file.try_clone()?;

    let mut child = Command::new(&ch_bin)
        .args(&args)
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_file2))
        .kill_on_drop(false)
        .spawn()
        .map_err(|e| ChvmError::ProcessError(format!("failed to start cloud-hypervisor: {e}")))?;

    let pid = child.id().ok_or_else(|| {
        ChvmError::ProcessError("cloud-hypervisor exited immediately before PID could be read".into())
    })? as i32;

    // Spawn a background task to reap the child process (avoid zombie)
    tokio::spawn(async move {
        let _ = child.wait().await;
    });

    // Wait for API socket to be ready
    let socket_path = vm_dir.join("api.sock");
    wait_for_socket(&socket_path, Duration::from_secs(30)).await?;

    info!(pid = pid, "Cloud Hypervisor started");
    Ok(pid)
}

/// Wait for a Unix socket to become available.
async fn wait_for_socket(path: &Path, timeout: Duration) -> Result<()> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match UnixStream::connect(path).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                if tokio::time::Instant::now() > deadline {
                    return Err(ChvmError::ProcessError(
                        format!("timeout waiting for API socket: {}", path.display())
                    ));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Find the Cloud Hypervisor binary path.
pub fn find_ch_binary() -> Option<String> {
    if let Ok(bin) = std::env::var("CHVM_CH_BIN") {
        if Path::new(&bin).exists() {
            return Some(bin);
        }
    }

    crate::utils::find_command("cloud-hypervisor", &[
        "/usr/local/bin/cloud-hypervisor",
        "/usr/bin/cloud-hypervisor",
    ])
}
