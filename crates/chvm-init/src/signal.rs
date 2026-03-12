use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use nix::sys::signal::Signal;
use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

static SHUTDOWN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SIGTERM_TIMEOUT: Duration = Duration::from_secs(3);
const SIGKILL_TIMEOUT: Duration = Duration::from_secs(1);

/// Reap zombie child processes using SIGCHLD.
/// Blocks on sigwait for SIGCHLD, then reaps all available children.
pub fn reap_zombies() {
    // Block SIGCHLD so we can wait for it with sigwait
    let mut mask = nix::sys::signal::SigSet::empty();
    mask.add(nix::sys::signal::Signal::SIGCHLD);
    let _ =
        nix::sys::signal::sigprocmask(nix::sys::signal::SigmaskHow::SIG_BLOCK, Some(&mask), None);

    loop {
        // Wait for SIGCHLD signal
        let _ = mask.wait();

        // Reap all available children (there may be multiple)
        while let Ok(WaitStatus::Exited(pid, _)) | Ok(WaitStatus::Signaled(pid, _, _)) =
            waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG))
        {
            println!("[init] Reaped child process {pid}");
        }
    }
}

/// Non-blocking reap of all zombie children. Returns true if no children remain.
fn reap_all_children() -> bool {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, code)) => {
                println!("[init] Reaped child process {pid} (exited {code})");
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                println!("[init] Reaped child process {pid} (killed by {sig})");
            }
            Ok(WaitStatus::StillAlive) => return false,
            Err(_) => return true,
            _ => continue,
        }
    }
}

/// Poll until all children are reaped or timeout expires. Returns true if none remain.
fn wait_for_children(timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if reap_all_children() {
            return true;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    false
}

/// Send SIGTERM to all children, wait up to 3s, then SIGKILL stragglers.
fn terminate_all_children() {
    println!("[init] Sending SIGTERM to all child processes...");
    let _ = nix::sys::signal::kill(Pid::from_raw(-1), Signal::SIGTERM);

    if wait_for_children(SIGTERM_TIMEOUT) {
        return;
    }

    println!("[init] Sending SIGKILL to remaining child processes...");
    let _ = nix::sys::signal::kill(Pid::from_raw(-1), Signal::SIGKILL);

    wait_for_children(SIGKILL_TIMEOUT);
}

/// Power off the VM.
pub fn poweroff(exit_code: i32) -> ! {
    // Prevent multiple concurrent shutdowns
    // if another thread has already initiated shutdown, just wait indefinitely
    if SHUTDOWN_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        loop {
            unsafe {
                libc::pause();
            }
        }
    }

    println!("[init] Powering off (exit code: {exit_code})...");

    // Terminate all child processes before unmounting
    terminate_all_children();

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::Arc;

    /// Fork a child that exits immediately, then verify reap_all_children() collects it.
    #[test]
    fn test_reap_all_children_reaps_exited_child() {
        match unsafe { libc::fork() } {
            0 => {
                // Child: exit immediately
                unsafe { libc::_exit(42) };
            }
            pid if pid > 0 => {
                // Parent: give child time to exit and become a zombie
                std::thread::sleep(Duration::from_millis(100));
                reap_all_children();
                // After reaping, waitpid for that specific pid should fail (already reaped)
                let result = waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG));
                assert!(result.is_err(), "child should already be reaped");
            }
            _ => panic!("fork failed"),
        }
    }

    /// Fork a child that sleeps, kill it with SIGKILL, then verify reap_all_children() collects it.
    #[test]
    fn test_reap_all_children_reaps_signaled_child() {
        match unsafe { libc::fork() } {
            0 => {
                // Child: sleep forever
                loop {
                    unsafe { libc::pause() };
                }
            }
            pid if pid > 0 => {
                // Parent: kill the child, then reap
                let _ = nix::sys::signal::kill(Pid::from_raw(pid), Signal::SIGKILL);
                std::thread::sleep(Duration::from_millis(100));
                reap_all_children();
                let result = waitpid(Pid::from_raw(pid), Some(WaitPidFlag::WNOHANG));
                assert!(result.is_err(), "child should already be reaped");
            }
            _ => panic!("fork failed"),
        }
    }

    /// Only one thread should win the shutdown guard.
    #[test]
    fn test_shutdown_guard_only_one_winner() {
        let flag = Arc::new(AtomicBool::new(false));
        let winners = Arc::new(AtomicUsize::new(0));

        let mut handles = vec![];
        for _ in 0..10 {
            let flag = Arc::clone(&flag);
            let winners = Arc::clone(&winners);
            handles.push(std::thread::spawn(move || {
                if !flag.swap(true, Ordering::SeqCst) {
                    winners.fetch_add(1, Ordering::SeqCst);
                }
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        assert_eq!(winners.load(Ordering::SeqCst), 1);
    }
}
