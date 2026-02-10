use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

/// Reap zombie child processes using SIGCHLD.
/// Blocks on sigwait for SIGCHLD, then reaps all available children.
pub fn reap_zombies() {
    // Block SIGCHLD so we can wait for it with sigwait
    let mut mask = nix::sys::signal::SigSet::empty();
    mask.add(nix::sys::signal::Signal::SIGCHLD);
    let _ = nix::sys::signal::sigprocmask(
        nix::sys::signal::SigmaskHow::SIG_BLOCK,
        Some(&mask),
        None,
    );

    loop {
        // Wait for SIGCHLD signal
        let _ = mask.wait();

        // Reap all available children (there may be multiple)
        while let Ok(WaitStatus::Exited(pid, _)) | Ok(WaitStatus::Signaled(pid, _, _)) = waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            println!("[init] Reaped child process {pid}");
        }
    }
}

/// Power off the VM.
pub fn poweroff(exit_code: i32) -> ! {
    println!("[init] Powering off (exit code: {exit_code})...");

    // Sync filesystems
    unsafe { libc::sync(); }

    // Unmount in reverse order
    for path in &["/mnt", "/run", "/tmp", "/dev/pts"] {
        let _ = nix::mount::umount2(*path, nix::mount::MntFlags::MNT_DETACH);
    }

    // Try ACPI poweroff
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/sys/power/state") {
        use std::io::Write;
        println!("[init] Triggering ACPI poweroff...");
        let _ = f.write_all(b"poweroff");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Try sysrq poweroff
    if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open("/proc/sysrq-trigger") {
        use std::io::Write;
        println!("[init] Triggering sysrq poweroff...");
        let _ = f.write_all(b"o");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Unmount remaining
    for path in &["/dev", "/sys", "/proc"] {
        let _ = nix::mount::umount2(*path, nix::mount::MntFlags::MNT_DETACH);
    }

    // Reboot syscall with POWER_OFF
    println!("[init] Triggering reboot syscall poweroff...");
    unsafe {
        libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF);
    }

    // Should not reach here
    println!("[init] Power off failed, halting...");
    loop {
        unsafe { libc::pause(); }
    }
}
