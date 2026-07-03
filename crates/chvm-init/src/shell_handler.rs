use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::Command;

/// In-band resize escape: \x01RSZ (SOH + "RSZ").
/// NOTE: Keep in sync with RESIZE_MAGIC in chvm-lib/src/vsock/protocol.rs
/// NOTE: see protocol.rs for known limitation with binary stdin data.
const RESIZE_MAGIC: [u8; 4] = [0x01, 0x52, 0x53, 0x5a];
const RESIZE_MSG_LEN: usize = 8;

/// Apply a terminal window size to a PTY master fd.
fn apply_winsize(master_fd: i32, rows: u16, cols: u16) {
    let ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    unsafe {
        if libc::ioctl(master_fd, libc::TIOCSWINSZ, &ws) == -1 {
            eprintln!("TIOCSWINSZ failed: {}", std::io::Error::last_os_error());
        }
    }
}

/// State-machine parser that detects in-band resize messages in a byte stream.
struct ResizeParser {
    buf: [u8; RESIZE_MSG_LEN],
    pos: usize,
}

impl ResizeParser {
    fn new() -> Self {
        Self {
            buf: [0u8; RESIZE_MSG_LEN],
            pos: 0,
        }
    }

    /// Feed a single byte. Returns an action: Forward, Resize, or Buffered.
    /// When a partial magic match fails, returns the buffered bytes via `flush`
    /// followed by re-checking the current byte.
    fn feed(&mut self, byte: u8, out: &mut Vec<u8>) -> Option<(u16, u16)> {
        if self.pos < 4 {
            // Matching magic bytes
            if byte == RESIZE_MAGIC[self.pos] {
                self.buf[self.pos] = byte;
                self.pos += 1;
                return None;
            }
            // Mismatch: flush buffered magic bytes as data
            out.extend_from_slice(&self.buf[..self.pos]);
            self.pos = 0;
            // Check if current byte starts a new match
            if byte == RESIZE_MAGIC[0] {
                self.buf[0] = byte;
                self.pos = 1;
            } else {
                out.push(byte);
            }
            None
        } else {
            // Accumulating payload (rows + cols)
            self.buf[self.pos] = byte;
            self.pos += 1;
            if self.pos == RESIZE_MSG_LEN {
                let rows = u16::from_be_bytes([self.buf[4], self.buf[5]]);
                let cols = u16::from_be_bytes([self.buf[6], self.buf[7]]);
                self.pos = 0;
                if rows == 0 || cols == 0 {
                    None
                } else {
                    Some((rows, cols))
                }
            } else {
                None
            }
        }
    }
}

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

    // Extract raw fd before cloning so we can use it in pre_exec
    let slave_raw_fd = slave.as_raw_fd();

    let (slave_in, slave_out, slave_err) =
        match (slave.try_clone(), slave.try_clone(), slave.try_clone()) {
            (Ok(a), Ok(b), Ok(c)) => (a, b, c),
            _ => {
                let mut w = &file;
                let _ = writeln!(w, "Failed to clone PTY slave fd");
                return;
            }
        };
    cmd.stdin(slave_in);
    cmd.stdout(slave_out);
    cmd.stderr(slave_err);

    // Create a new session and set the PTY slave as the controlling terminal
    unsafe {
        cmd.pre_exec(move || {
            // Create a NEW session, make bash the session leader
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // TIOCSCTTY tells the kernel that this PTY slave is now the controlling terminal of the session. This is necessary for signal handling (e.g. Ctrl-C) to work correctly.
            if libc::ioctl(slave_raw_fd, libc::TIOCSCTTY, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

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
    let (mut master_clone, file_clone) = match (master.try_clone(), file.try_clone()) {
        (Ok(m), Ok(f)) => (m, f),
        _ => {
            let mut w = &file;
            let _ = writeln!(w, "Failed to clone fd for proxy");
            return;
        }
    };

    // vsock -> pty (with resize message parsing)
    let master_raw_fd = master_clone.as_raw_fd();
    let t1 = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        let mut parser = ResizeParser::new();
        let mut forward_buf = Vec::with_capacity(4096);
        loop {
            match (&file_clone).read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    forward_buf.clear();
                    for &byte in &buf[..n] {
                        if let Some((rows, cols)) = parser.feed(byte, &mut forward_buf) {
                            // Flush any pending data before applying resize
                            if !forward_buf.is_empty() {
                                if master_clone.write_all(&forward_buf).is_err() {
                                    return;
                                }
                                forward_buf.clear();
                            }
                            apply_winsize(master_raw_fd, rows, cols);
                        }
                    }
                    if !forward_buf.is_empty() && master_clone.write_all(&forward_buf).is_err() {
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
        let master_fd = libc::open(
            c"/dev/ptmx".as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        );
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
        let slave_fd = libc::open(slave_path.as_ptr() as _, libc::O_RDWR | libc::O_CLOEXEC);
        if slave_fd < 0 {
            libc::close(master_fd);
            return Err(format!("open slave: {}", std::io::Error::last_os_error()).into());
        }

        let master = std::fs::File::from_raw_fd(master_fd);
        let slave = std::fs::File::from_raw_fd(slave_fd);

        Ok((master, slave))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resize_constants_match_protocol() {
        // These bytes must match encode_resize() output in chvm-lib/src/vsock/protocol.rs
        // If this test fails, the two crates have diverged.
        assert_eq!(RESIZE_MAGIC, [0x01, 0x52, 0x53, 0x5a]); // SOH + "RSZ"
        assert_eq!(RESIZE_MSG_LEN, 8); // 4 magic + 2 rows + 2 cols
    }

    #[test]
    fn test_parser_passes_through_normal_data() {
        let mut parser = ResizeParser::new();
        let mut out = Vec::new();
        let data = b"hello world";
        for &b in data {
            assert!(parser.feed(b, &mut out).is_none());
        }
        assert_eq!(out, data);
    }

    #[test]
    fn test_parser_detects_resize_message() {
        let mut parser = ResizeParser::new();
        let mut out = Vec::new();
        let msg: [u8; 8] = [0x01, 0x52, 0x53, 0x5a, 0x00, 24, 0x00, 80];
        let mut resize = None;
        for &b in &msg {
            if let Some(r) = parser.feed(b, &mut out) {
                resize = Some(r);
            }
        }
        assert_eq!(resize, Some((24, 80)));
        assert!(out.is_empty());
    }

    #[test]
    fn test_parser_handles_partial_magic_mismatch() {
        let mut parser = ResizeParser::new();
        let mut out = Vec::new();
        // Start of magic then diverge: 0x01, 0x52, then 0x41 (not 0x53)
        let data = [0x01, 0x52, 0x41];
        for &b in &data {
            parser.feed(b, &mut out);
        }
        assert_eq!(out, &[0x01, 0x52, 0x41]);
    }

    #[test]
    fn test_parser_handles_split_across_reads() {
        let mut parser = ResizeParser::new();
        let msg: [u8; 8] = [0x01, 0x52, 0x53, 0x5a, 0x00, 30, 0x00, 120];

        // Feed first 3 bytes
        let mut out = Vec::new();
        for &b in &msg[..3] {
            assert!(parser.feed(b, &mut out).is_none());
        }
        assert!(out.is_empty());

        // Feed remaining 5 bytes
        let mut resize = None;
        for &b in &msg[3..] {
            if let Some(r) = parser.feed(b, &mut out) {
                resize = Some(r);
            }
        }
        assert_eq!(resize, Some((30, 120)));
        assert!(out.is_empty());
    }

    #[test]
    fn test_parser_0x01_in_normal_data() {
        let mut parser = ResizeParser::new();
        let mut out = Vec::new();
        // 0x01 followed by non-magic byte
        let data = [0x41, 0x01, 0x42];
        for &b in &data {
            parser.feed(b, &mut out);
        }
        assert_eq!(out, &[0x41, 0x01, 0x42]);
    }

    #[test]
    fn test_parser_back_to_back_resize() {
        let mut parser = ResizeParser::new();
        let mut out = Vec::new();
        let msg1: [u8; 8] = [0x01, 0x52, 0x53, 0x5a, 0x00, 24, 0x00, 80];
        let msg2: [u8; 8] = [0x01, 0x52, 0x53, 0x5a, 0x00, 40, 0x00, 132];

        let mut resizes = Vec::new();
        for &b in msg1.iter().chain(msg2.iter()) {
            if let Some(r) = parser.feed(b, &mut out) {
                resizes.push(r);
            }
        }
        assert_eq!(resizes, vec![(24, 80), (40, 132)]);
        assert!(out.is_empty());
    }

    #[test]
    fn test_parser_data_around_resize() {
        let mut parser = ResizeParser::new();
        let mut out = Vec::new();
        let msg: [u8; 8] = [0x01, 0x52, 0x53, 0x5a, 0x00, 24, 0x00, 80];

        // "AB" + resize + "CD"
        let mut resizes = Vec::new();
        for &b in [0x41, 0x42]
            .iter()
            .chain(msg.iter())
            .chain([0x43, 0x44].iter())
        {
            if let Some(r) = parser.feed(b, &mut out) {
                resizes.push(r);
            }
        }
        assert_eq!(resizes, vec![(24, 80)]);
        assert_eq!(out, &[0x41, 0x42, 0x43, 0x44]);
    }
}
