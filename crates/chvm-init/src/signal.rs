use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use nix::sys::signal::Signal;
use std::sync::mpsc;

use nix::sys::wait::{waitpid, WaitPidFlag, WaitStatus};
use nix::unistd::Pid;

static SHUTDOWN_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Registry that lets the single PID-1 reaper hand a reaped child's real exit
/// code to a waiting exec handler. Without this, the reaper's `waitpid(-1)`
/// steals the exec child before the handler can `wait()` it, so the handler
/// gets ECHILD and reports a bogus exit code.
struct ExecReaper {
    inner: Mutex<HashMap<i32, i32>>, // pid -> exit code, once reaped
    cv: Condvar,
}

fn exec_reaper() -> &'static ExecReaper {
    static R: OnceLock<ExecReaper> = OnceLock::new();
    R.get_or_init(|| ExecReaper {
        inner: Mutex::new(HashMap::new()),
        cv: Condvar::new(),
    })
}

/// Record a reaped child's exit code and wake any waiter. Called by the reaper.
fn publish_exec_exit(pid: i32, code: i32) {
    let reaper = exec_reaper();
    let mut map = reaper.inner.lock().unwrap_or_else(|e| e.into_inner());
    // Bounded defensively: unwaited orphan reaps (rare in this guest) can't grow
    // the map without limit.
    if map.len() > 4096 {
        map.clear();
    }
    map.insert(pid, code);
    reaper.cv.notify_all();
}

/// Block until the reaper publishes `pid`'s exit code, or `timeout` elapses.
/// Returns None on timeout (caller falls back to a default code).
pub fn wait_exec_exit(pid: i32, timeout: Duration) -> Option<i32> {
    let reaper = exec_reaper();
    let mut map = reaper.inner.lock().unwrap_or_else(|e| e.into_inner());
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(code) = map.remove(&pid) {
            return Some(code);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        let (guard, res) = reaper
            .cv
            .wait_timeout(map, remaining)
            .unwrap_or_else(|e| e.into_inner());
        map = guard;
        if res.timed_out() {
            if let Some(code) = map.remove(&pid) {
                return Some(code);
            }
            return None;
        }
    }
}
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const SIGTERM_TIMEOUT: Duration = Duration::from_secs(3);
const SIGKILL_TIMEOUT: Duration = Duration::from_secs(1);

use crate::mycelium::MyceliumConfig;

const MYCELIUM_RESTART_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
const MYCELIUM_SPAWN_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);
const MYCELIUM_SPAWN_RETRIES: u32 = 5;

/// Block SIGCHLD on the calling thread. Must be called on the main thread
/// before spawning any children so that SIGCHLD is inherited as blocked by
/// all threads, ensuring the reaper thread can reliably catch it via sigtimedwait.
pub fn block_sigchld() {
    let mut mask = nix::sys::signal::SigSet::empty();
    mask.add(nix::sys::signal::Signal::SIGCHLD);
    if let Err(e) =
        nix::sys::signal::sigprocmask(nix::sys::signal::SigmaskHow::SIG_BLOCK, Some(&mask), None)
    {
        eprintln!("[init] Warning: failed to block SIGCHLD: {e}");
    }
}

/// Reap zombie child processes using SIGCHLD.
pub fn reap_zombies(
    main_pid: i32,
    main_exit_tx: mpsc::Sender<i32>,
    mycelium: Option<(i32, std::process::Child, MyceliumConfig)>,
) {
    let (mut mycelium_pid, mut mycelium_child, mycelium_config) = match mycelium {
        Some((pid, child, config)) => (Some(pid), Some(child), Some(config)),
        None => (None, None, None),
    };

    // Pending restart: set when mycelium dies, cleared after respawn
    let mut restart_after: Option<std::time::Instant> = None;
    let mut spawn_attempts: u32 = 0;

    let mut mask = nix::sys::signal::SigSet::empty();
    mask.add(nix::sys::signal::Signal::SIGCHLD);

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
        loop {
            let (raw_pid, exit_code) = match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG))
            {
                Ok(WaitStatus::Exited(pid, code)) => (pid.as_raw(), code),
                Ok(WaitStatus::Signaled(pid, sig, _)) => (pid.as_raw(), 128 + sig as i32),
                _ => break,
            };

            if raw_pid == main_pid {
                let _ = main_exit_tx.send(exit_code);
            } else if mycelium_pid == Some(raw_pid) {
                eprintln!("[init] Mycelium (PID {raw_pid}) exited, scheduling restart...");
                mycelium_child.take();
                mycelium_pid = None;
                restart_after = Some(std::time::Instant::now() + MYCELIUM_RESTART_DELAY);
            } else {
                // Hand the exit code to any exec handler waiting on this child,
                // and also record it in case the wait races slightly behind.
                publish_exec_exit(raw_pid, exit_code);
            }
        }

        // Handle pending mycelium restart without blocking the reap loop
        let ready = restart_after.is_some_and(|d| std::time::Instant::now() >= d);
        if !ready {
            continue;
        }

        let Some(ref config) = mycelium_config else {
            restart_after = None;
            continue;
        };

        spawn_attempts += 1;
        if let Some((pid, child)) = crate::mycelium::spawn_mycelium(config) {
            mycelium_pid = Some(pid);
            mycelium_child = Some(child);
            restart_after = None;
            spawn_attempts = 0;
        } else if spawn_attempts < MYCELIUM_SPAWN_RETRIES {
            eprintln!(
                "[init] Mycelium spawn attempt {spawn_attempts}/{MYCELIUM_SPAWN_RETRIES} failed, retrying..."
            );
            restart_after = Some(std::time::Instant::now() + MYCELIUM_SPAWN_RETRY_DELAY);
        } else {
            eprintln!(
                "[init] Mycelium spawn failed after {MYCELIUM_SPAWN_RETRIES} attempts, giving up"
            );
            restart_after = None;
            spawn_attempts = 0;
        }
    }
}

/// Non-blocking reap of all zombie children. Returns true if no children remain.
fn reap_all_children() -> bool {
    loop {
        match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
            Ok(WaitStatus::Exited(pid, code)) => {
                publish_exec_exit(pid.as_raw(), code);
            }
            Ok(WaitStatus::Signaled(pid, sig, _)) => {
                publish_exec_exit(pid.as_raw(), 128 + sig as i32);
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
