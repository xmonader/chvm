// NOTE: Keep in sync with chvm-lib/src/vsock/protocol.rs
// (chvm-init is a separate guest binary and can't depend on chvm-lib).
const VSOCK_PORT_EXEC: u32 = 1024;
const VSOCK_PORT_SHELL: u32 = 1025;
const VSOCK_CID_ANY: u32 = 0xFFFFFFFF;

/// Run a vsock server on the given port, dispatching connections to handler.
fn run_server(port: u32, name: &str, handler: fn(i32)) {
    let fd = match create_vsock_listener(port) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("[init] Warning: Failed to create {name} vsock: {e}");
            return;
        }
    };

    println!("[init] {name} server listening on vsock port {port}");

    loop {
        let conn_fd = unsafe { libc::accept(fd, std::ptr::null_mut(), std::ptr::null_mut()) };
        if conn_fd < 0 {
            continue;
        }
        std::thread::spawn(move || handler(conn_fd));
    }
}

pub fn run_exec_server() {
    run_server(VSOCK_PORT_EXEC, "Exec", crate::exec_handler::handle_exec);
}

pub fn run_shell_server() {
    run_server(
        VSOCK_PORT_SHELL,
        "Shell",
        crate::shell_handler::handle_shell,
    );
}

fn create_vsock_listener(port: u32) -> Result<i32, String> {
    unsafe {
        let fd = libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM, 0);
        if fd < 0 {
            return Err(format!(
                "socket() failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        // Build sockaddr_vm
        let mut addr: libc::sockaddr_vm = std::mem::zeroed();
        addr.svm_family = libc::AF_VSOCK as u16;
        addr.svm_cid = VSOCK_CID_ANY;
        addr.svm_port = port;

        let addr_ptr = &addr as *const libc::sockaddr_vm as *const libc::sockaddr;
        let addr_len = std::mem::size_of::<libc::sockaddr_vm>() as libc::socklen_t;

        if libc::bind(fd, addr_ptr, addr_len) < 0 {
            libc::close(fd);
            return Err(format!(
                "bind() failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        if libc::listen(fd, 5) < 0 {
            libc::close(fd);
            return Err(format!(
                "listen() failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(fd)
    }
}
