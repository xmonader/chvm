use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

use crate::mycelium::MyceliumConfig;

const MYCELIUM_RESTART_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
const MYCELIUM_SPAWN_RETRIES: u32 = 5;

/// Reap zombie child processes using SIGCHLD.
/// Blocks on sigwait for SIGCHLD, then reaps all available children.
/// If a mycelium config is provided, automatically restarts mycelium when
/// its process is reaped.
pub fn reap_zombies(mycelium: Option<(u32, MyceliumConfig)>) {
    let mut mycelium_pid = mycelium.as_ref().map(|(pid, _)| *pid);
    let mycelium_config = mycelium.map(|(_, config)| config);

    // Pending restart: set when mycelium dies, cleared after respawn
    let mut restart_after: Option<std::time::Instant> = None;

    // Block SIGCHLD so we can wait for it with sigwait
    let mut mask = nix::sys::signal::SigSet::empty();
    mask.add(nix::sys::signal::Signal::SIGCHLD);
    if let Err(e) = nix::sys::signal::sigprocmask(
        nix::sys::signal::SigmaskHow::SIG_BLOCK,
        Some(&mask),
        None,
    ) {
        eprintln!("[init] Fatal: failed to block SIGCHLD: {e}");
        return;
    }

    loop {
        // Use sigtimedwait so we can wake up to handle pending restarts
        let timeout = if restart_after.is_some() {
            // Check every 500ms for restart deadline
            libc::timespec {
                tv_sec: 0,
                tv_nsec: 500_000_000,
            }
        } else {
            // No pending restart — block indefinitely (large timeout)
            libc::timespec {
                tv_sec: 3600,
                tv_nsec: 0,
            }
        };

        unsafe {
            libc::sigtimedwait(mask.as_ref(), std::ptr::null_mut(), &timeout);
        }

        // Reap all available children (there may be multiple)
        while let Ok(WaitStatus::Exited(pid, _)) | Ok(WaitStatus::Signaled(pid, _, _)) =
            waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG))
        {
            let raw_pid = pid.as_raw() as u32;

            if mycelium_pid == Some(raw_pid) {
                eprintln!("[init] Mycelium (PID {raw_pid}) exited, scheduling restart...");
                mycelium_pid = None;
                restart_after = Some(std::time::Instant::now() + MYCELIUM_RESTART_DELAY);
            } else {
                println!("[init] Reaped child process {pid}");
            }
        }

        // Handle pending mycelium restart without blocking the reap loop
        if let Some(deadline) = restart_after {
            if std::time::Instant::now() >= deadline {
                restart_after = None;
                if let Some(ref config) = mycelium_config {
                    mycelium_pid = try_spawn_mycelium(config);
                }
            }
        }
    }
}

/// Try to spawn mycelium with retries on failure.
fn try_spawn_mycelium(config: &MyceliumConfig) -> Option<u32> {
    for attempt in 0..MYCELIUM_SPAWN_RETRIES {
        if let Some(pid) = crate::mycelium::spawn_mycelium(config) {
            return Some(pid);
        }
        eprintln!(
            "[init] Mycelium spawn attempt {}/{MYCELIUM_SPAWN_RETRIES} failed, retrying...",
            attempt + 1
        );
        std::thread::sleep(MYCELIUM_RESTART_DELAY);
    }
    eprintln!("[init] Mycelium spawn failed after {MYCELIUM_SPAWN_RETRIES} attempts");
    None
}

/// Power off the VM.
pub fn poweroff(exit_code: i32) -> ! {
    println!("[init] Powering off (exit code: {exit_code})...");

    // Sync filesystems
    unsafe {
        libc::sync();
    }

    // Unmount in reverse order
    for path in &["/mnt", "/run", "/tmp", "/dev/pts"] {
        let _ = nix::mount::umount2(*path, nix::mount::MntFlags::MNT_DETACH);
    }

    // Try ACPI poweroff
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .open("/sys/power/state")
    {
        use std::io::Write;
        println!("[init] Triggering ACPI poweroff...");
        let _ = f.write_all(b"poweroff");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    // Try sysrq poweroff
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .write(true)
        .open("/proc/sysrq-trigger")
    {
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
        unsafe {
            libc::pause();
        }
    }
}
