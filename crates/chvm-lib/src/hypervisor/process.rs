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
    push_arg(
        &mut args,
        "--memory",
        format!("size={}M,shared=on", config.memory_mb),
    );
    push_arg(&mut args, "--kernel", &config.kernel.path);
    push_arg(&mut args, "--cmdline", &config.kernel.cmdline);

    if let Some(ref initrd) = config.kernel.initrd_path {
        push_arg(&mut args, "--initramfs", initrd);
    }

    let mut fs_entries = Vec::new();
    match config.rootfs.rootfs_type {
        RootfsType::VirtioFs => {
            fs_entries.push(format!("tag=rootfs,socket={}", config.rootfs.source));
        }
        RootfsType::VirtioBlock => {
            // image_type is explicit and mandatory: cloud-hypervisor v52+
            // auto-detects raw and disables sector-0 writes, breaking ext4
            // root filesystems; qcow2 overlays must be declared as well
            // (auto-detection is deprecated). Format comes from the storage
            // backend (raw is the default when absent).
            //
            // qcow2 overlays reference a backing file, and cloud-hypervisor
            // v54 only follows backing chains when `backing_files=on` is
            // given (the default is off, which rejects any qcow2 with a
            // backing file as MaxNestingDepthExceeded). Raw images have no
            // chain, so the flag is only meaningful (and only added) for
            // qcow2.
            let format = config.rootfs.block_format.as_deref().unwrap_or("raw");
            let disk = if format == "qcow2" {
                format!(
                    "path={},image_type={format},backing_files=on",
                    config.rootfs.source
                )
            } else {
                format!("path={},image_type={format}", config.rootfs.source)
            };
            push_arg(&mut args, "--disk", disk);
        }
    }

    // Guest init stdio goes to console.log through the virtio console (hvc0).
    // The serial port is a PTY allocated by cloud-hypervisor: kernel messages
    // and panics land there even if guest init or the vsock path is broken,
    // and `chvm console` attaches to it as a rescue shell.
    push_arg(&mut args, "--console", format!("file={log_path}"));
    push_arg(&mut args, "--serial", "pty");

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
            fs_entries.push(format!("tag={tag},socket={}", mount.source));
        }
    }

    if !fs_entries.is_empty() {
        args.push("--fs".into());
        args.extend(fs_entries);
    }

    if config.vsock_cid > 0 {
        let vsock_sock = vm_dir.join("vsock.sock");
        push_arg(
            &mut args,
            "--vsock",
            format!("cid={},socket={}", config.vsock_cid, vsock_sock.display()),
        );
    }

    push_arg(&mut args, "--log-file", ch_log_path.to_string_lossy());

    args
}

/// Launch a Cloud Hypervisor process and return its PID.
pub async fn launch(config: &VmConfig, vm_dir: &Path) -> Result<i32> {
    let ch_bin = find_ch_binary().ok_or_else(|| {
        ChvmError::HypervisorNotFound(
            "cloud-hypervisor not found. Install it or set CHVM_CH_BIN env var.".into(),
        )
    })?;

    let args = build_args(config, vm_dir);

    debug!(bin = %ch_bin, args = ?args, "Launching cloud-hypervisor");

    let default_launch_log = vm_dir.join("console.log");
    let default_launch_log_str = default_launch_log.to_string_lossy();
    let launch_log_path = config
        .log_path
        .as_deref()
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
        ChvmError::ProcessError(
            "cloud-hypervisor exited immediately before PID could be read".into(),
        )
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
                    return Err(ChvmError::ProcessError(format!(
                        "timeout waiting for API socket: {}",
                        path.display()
                    )));
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

    crate::utils::find_command(
        "cloud-hypervisor",
        &[
            "/usr/local/bin/cloud-hypervisor",
            "/usr/bin/cloud-hypervisor",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::state::{InitConfig, KernelConfig, MountConfig, NetworkConfig, RootfsConfig};

    fn sample_vm_config(rootfs_type: RootfsType, mounts: Vec<MountConfig>) -> VmConfig {
        VmConfig {
            id: "vm1".into(),
            name: "vm1".into(),
            vcpu: 1,
            memory_mb: 512,
            kernel: KernelConfig {
                path: "/kernel".into(),
                cmdline: "console=hvc0".into(),
                initrd_path: None,
            },
            rootfs: RootfsConfig {
                source: "/rootfs.sock".into(),
                rootfs_type,
                read_only: false,
                block_disk_size_bytes: None,
                writable_quota_bytes: None,
                block_format: None,
            },
            network: NetworkConfig {
                mode: crate::vm::state::NetworkMode::None,
                tap_device: None,
                mac_address: None,
                ip_address: None,
                gateway: None,
                port_forwards: vec![],
                mycelium_peers: vec![],
                mycelium_key: None,
            },
            init: InitConfig {
                cmd: vec!["/bin/sh".into()],
                env: vec![],
                working_dir: Some("/".into()),
            },
            mounts,
            log_path: None,
            vsock_cid: 3,
            state_dir: Some("/vm".into()),
        }
    }

    #[test]
    fn build_args_uses_single_fs_flag_for_rootfs_and_mounts() {
        let config = sample_vm_config(
            RootfsType::VirtioFs,
            vec![MountConfig {
                source: "/mount.sock".into(),
                target: "/mnt/data".into(),
                mount_type: Some("virtiofs".into()),
                read_only: false,
                tag: Some("mount0".into()),
            }],
        );

        let args = build_args(&config, Path::new("/vm"));
        let fs_positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(idx, arg)| (arg == "--fs").then_some(idx))
            .collect();

        assert_eq!(fs_positions.len(), 1);
        let fs_index = fs_positions[0];
        assert_eq!(args[fs_index + 1], "tag=rootfs,socket=/rootfs.sock");
        assert_eq!(args[fs_index + 2], "tag=mount0,socket=/mount.sock");
    }

    #[test]
    fn build_args_marks_block_rootfs_as_raw_image() {
        let config = sample_vm_config(RootfsType::VirtioBlock, vec![]);

        let args = build_args(&config, Path::new("/vm"));
        let disk_positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(idx, arg)| (arg == "--disk").then_some(idx))
            .collect();

        assert_eq!(disk_positions.len(), 1);
        assert_eq!(
            args[disk_positions[0] + 1],
            "path=/rootfs.sock,image_type=raw"
        );
    }

    #[test]
    fn build_args_enables_backing_files_for_qcow2_overlays() {
        // cloud-hypervisor v54 rejects qcow2 images with a backing file
        // unless backing_files=on is passed (default is off).
        let mut config = sample_vm_config(RootfsType::VirtioBlock, vec![]);
        config.rootfs.block_format = Some("qcow2".into());

        let args = build_args(&config, Path::new("/vm"));
        let disk_positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(idx, arg)| (arg == "--disk").then_some(idx))
            .collect();

        assert_eq!(disk_positions.len(), 1);
        assert_eq!(
            args[disk_positions[0] + 1],
            "path=/rootfs.sock,image_type=qcow2,backing_files=on"
        );
    }

    #[test]
    fn build_args_uses_serial_pty_for_rescue_console() {
        let config = sample_vm_config(RootfsType::VirtioFs, vec![]);

        let args = build_args(&config, Path::new("/vm"));
        let serial_positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(idx, arg)| (arg == "--serial").then_some(idx))
            .collect();

        assert_eq!(serial_positions.len(), 1);
        assert_eq!(args[serial_positions[0] + 1], "pty");
    }

    #[test]
    fn build_args_keeps_console_log_for_guest_stdio() {
        let config = sample_vm_config(RootfsType::VirtioFs, vec![]);

        let args = build_args(&config, Path::new("/vm"));
        let console_positions: Vec<_> = args
            .iter()
            .enumerate()
            .filter_map(|(idx, arg)| (arg == "--console").then_some(idx))
            .collect();

        assert_eq!(console_positions.len(), 1);
        assert_eq!(args[console_positions[0] + 1], "file=/vm/console.log");
    }
}
