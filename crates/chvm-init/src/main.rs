mod cmdline_parser;
mod devices;
mod exec_handler;
pub(crate) mod helpers;
mod mounts;
mod mycelium;
mod network;
mod rootfs;
mod serial_console;
mod shell_handler;
mod signal;
mod vsock_server;

use std::process::Command;

fn main() {
    if version_requested() {
        println!("chvm-init {}", env!("CHVM_FULL_VERSION"));
        return;
    }

    println!("[init] CHVM guest init starting...");

    // Set PATH early
    std::env::set_var("PATH", helpers::PATH_ENV);

    // Mount essential filesystems (fatal — /proc, /sys, /dev are required)
    if let Err(e) = mounts::mount_essential() {
        eprintln!("[init] Fatal: Failed to mount essential filesystems: {e}");
        signal::poweroff(1);
    }

    // Create device nodes
    if let Err(e) = devices::create_devices() {
        eprintln!("[init] Warning: Failed to create device nodes: {e}");
    }

    // Mount and switch to the configured rootfs unless we're already on it.
    if rootfs::should_mount_rootfs() {
        if let Err(e) = rootfs::mount_rootfs() {
            eprintln!("[init] Error mounting rootfs: {e}");
            signal::poweroff(1);
        }
    } else {
        println!("[init] Rootfs already mounted (block device boot)");
    }

    // Load kernel modules needed for vsock and virtiofs. This must run before
    // the vsock servers below (they bind AF_VSOCK) and before mount_volumes
    // (needs the virtiofs module).
    load_kernel_modules();

    // Block SIGCHLD on the main thread before spawning any threads that in
    // turn spawn children, so every thread (including the vsock servers)
    // inherits the blocked mask and the reaper can catch SIGCHLD reliably
    // via sigtimedwait.
    signal::block_sigchld();

    // Start vsock servers in background threads BEFORE configuring the
    // network. The exec/shell servers only depend on the vsock module (loaded
    // above), not on eth0. Binding them first guarantees chvm exec/attach can
    // connect even when network setup degrades (e.g. a leaseless bridge): a
    // DHCP attempt must never gate the vsock path.
    std::thread::spawn(vsock_server::run_exec_server);
    std::thread::spawn(vsock_server::run_shell_server);

    // Serial rescue console: a /bin/sh on /dev/ttyS0, reachable from the host
    // via `chvm console` even when the main command, networking, or the vsock
    // path are broken. Best-effort — never blocks boot.
    std::thread::spawn(serial_console::run_rescue_shell);

    // Setup network (best-effort; a leaseless bridge degrades to a no-IP warning)
    if let Err(e) = network::setup_network() {
        eprintln!("[init] Warning: Failed to setup network: {e}");
    }

    // Mount additional volumes (needs virtiofs module loaded)
    if let Err(e) = mounts::mount_volumes() {
        eprintln!("[init] Warning: Failed to mount volumes: {e}");
    }

    // Setup and spawn mycelium if configured
    let mycelium_state = match mycelium::setup_mycelium() {
        Ok(Some(config)) => {
            mycelium::spawn_mycelium(&config).map(|(pid, child)| (pid, child, config))
        }
        Ok(None) => None,
        Err(e) => {
            eprintln!("[init] Warning: Failed to setup mycelium: {e}");
            None
        }
    };

    // Load configuration from kernel cmdline
    let config = cmdline_parser::load_config();

    // Set hostname
    unsafe {
        libc::sethostname(c"chvm".as_ptr(), 4);
    }

    // Execute the main process
    let cmd_path = &config.cmd[0];
    let cmd_args = &config.cmd[1..];

    // Look up command in PATH if not absolute
    let resolved = if !cmd_path.starts_with('/') {
        helpers::find_binary(cmd_path, &[]).unwrap_or_else(|| cmd_path.clone())
    } else {
        cmd_path.clone()
    };

    println!("[init] Executing: {:?}", config.cmd);

    let mut cmd = Command::new(&resolved);
    cmd.args(cmd_args);

    // Set environment
    for env_var in &config.env {
        if let Some((key, value)) = env_var.split_once('=') {
            cmd.env(key, value);
        }
    }
    helpers::setup_standard_env(&mut cmd);

    // Set working directory
    if !config.working_dir.is_empty() {
        cmd.current_dir(&config.working_dir);
    }

    cmd.stdin(std::process::Stdio::inherit());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());

    // Setup signal forwarding
    let child = cmd.spawn();

    match child {
        Ok(child) => {
            let child_pid = child.id() as i32;

            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || signal::reap_zombies(child_pid, tx, mycelium_state));

            let exit_code = rx.recv().unwrap_or(1);
            println!("[init] Main process exited with code {exit_code}");
            signal::poweroff(exit_code);
        }
        Err(e) => {
            eprintln!("[init] Failed to start process: {e}");
            signal::poweroff(1);
        }
    }
}

fn version_requested() -> bool {
    std::env::args()
        .skip(1)
        .any(|arg| arg == "--version" || arg == "-V")
}

fn load_kernel_modules() {
    let modules_dir = std::path::Path::new("/lib/modules");
    if !modules_dir.exists() {
        return;
    }

    for module_file in helpers::GUEST_KERNEL_MODULES {
        let path = modules_dir.join(module_file);
        if !path.exists() {
            continue;
        }

        if let Ok(data) = std::fs::read(&path) {
            let result = unsafe {
                libc::syscall(
                    libc::SYS_init_module,
                    data.as_ptr(),
                    data.len(),
                    c"".as_ptr(),
                )
            };
            if result == 0 {
                println!("[init] Loaded module: {module_file}");
            } else {
                let errno = std::io::Error::last_os_error();
                // EEXIST means already loaded — not an error
                if errno.raw_os_error() != Some(libc::EEXIST) {
                    eprintln!("[init] Warning: Failed to load {module_file}: {errno}");
                }
            }
        }
    }
}
