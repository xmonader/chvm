use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
use std::process::Command;

pub fn handle_shell(fd: i32) {
    let file = unsafe { std::fs::File::from_raw_fd(fd) };

    // Find a shell
    let shell = ["/bin/bash", "/bin/ash", "/bin/sh"]
        .iter()
        .find(|s| std::path::Path::new(s).exists())
        .unwrap_or(&"/bin/sh");

    // Open a PTY
    let (master, slave) = match open_pty() {
        Ok(pair) => pair,
        Err(e) => {
            let mut w = &file;
            let _ = writeln!(w, "Failed to open PTY: {e}");
            return;
        }
    };

    // Start shell
    let mut cmd = Command::new(shell);
    cmd.arg("-i");
    crate::helpers::setup_standard_env(&mut cmd);
    cmd.env("PS1", "# ");
    cmd.current_dir("/root");

    cmd.stdin(slave.try_clone().unwrap());
    cmd.stdout(slave.try_clone().unwrap());
    cmd.stderr(slave.try_clone().unwrap());

    let _child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let mut w = &file;
            let _ = writeln!(w, "Failed to start shell: {e}");
            return;
        }
    };
    drop(slave);

    // Bidirectional proxy: vsock <-> pty
    let mut master_clone = master.try_clone().unwrap();
    let file_clone = file.try_clone().unwrap();

    // vsock -> pty
    let t1 = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match (&file_clone).read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if master_clone.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    // pty -> vsock
    let mut master2 = master;
    let mut file2 = file;
    let t2 = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match master2.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if file2.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let _ = t1.join();
    let _ = t2.join();
}

fn open_pty() -> Result<(std::fs::File, std::fs::File), Box<dyn std::error::Error>> {
    unsafe {
        let master_fd = libc::open(c"/dev/ptmx".as_ptr(), libc::O_RDWR | libc::O_NOCTTY);
        if master_fd < 0 {
            return Err(format!("open /dev/ptmx: {}", std::io::Error::last_os_error()).into());
        }

        // Get pts number
        let mut pts_num: libc::c_int = 0;
        if libc::ioctl(master_fd, libc::TIOCGPTN as _, &mut pts_num) < 0 {
            libc::close(master_fd);
            return Err("TIOCGPTN failed".into());
        }

        // Unlock
        let mut unlock: libc::c_int = 0;
        if libc::ioctl(master_fd, libc::TIOCSPTLCK as _, &mut unlock) < 0 {
            libc::close(master_fd);
            return Err("TIOCSPTLCK failed".into());
        }

        let slave_path = format!("/dev/pts/{pts_num}\0");
        let slave_fd = libc::open(slave_path.as_ptr() as _, libc::O_RDWR | libc::O_NOCTTY);
        if slave_fd < 0 {
            libc::close(master_fd);
            return Err(format!("open slave: {}", std::io::Error::last_os_error()).into());
        }

        let master = std::fs::File::from_raw_fd(master_fd);
        let slave = std::fs::File::from_raw_fd(slave_fd);

        Ok((master, slave))
    }
}
