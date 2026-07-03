use std::ffi::CString;
use std::net::Ipv4Addr;
use std::process::Command;

pub fn setup_network() -> Result<(), Box<dyn std::error::Error>> {
    let ip_cmd = crate::helpers::find_binary("ip", &["/sbin/ip", "/chvm-bin/ip"]);

    // Bring up loopback — use ioctl directly so it works even without iproute2
    bring_up_loopback();

    let cmdline = std::fs::read_to_string("/proc/cmdline").unwrap_or_default();
    let mut ip_addr = None;
    let mut gateway = None;
    let mut dns_servers = Vec::new();

    for part in cmdline.split_whitespace() {
        if let Some(val) = part.strip_prefix("chvm.ip=") {
            ip_addr = Some(val.to_string());
        } else if ip_addr.is_none() {
            if let Some(val) = part.strip_prefix("ip=") {
                ip_addr = Some(val.to_string());
            }
        }

        if let Some(val) = part.strip_prefix("chvm.gw=") {
            gateway = Some(val.to_string());
        } else if gateway.is_none() {
            if let Some(val) = part.strip_prefix("gw=") {
                gateway = Some(val.to_string());
            }
        }

        if let Some(val) = part.strip_prefix("chvm.dns=") {
            dns_servers.extend(
                val.split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(ToString::to_string),
            );
        }
    }

    if dns_servers.is_empty() {
        dns_servers = crate::helpers::DNS_SERVERS
            .iter()
            .map(|server| server.to_string())
            .collect();
    }

    // Bring up eth0 — prefer a real ip implementation when available.
    if !run_ip(&ip_cmd, &["link", "set", "eth0", "up"]) {
        bring_up_interface("eth0");
    }

    if let Some(ref addr) = ip_addr {
        println!(
            "[init] Configuring network: ip={addr} gw={}",
            gateway.as_deref().unwrap_or("none")
        );

        let configured_with_ip = run_ip(&ip_cmd, &["addr", "add", addr, "dev", "eth0"])
            && gateway
                .as_deref()
                .is_none_or(|gw| run_ip(&ip_cmd, &["route", "add", "default", "via", gw]));

        if !configured_with_ip {
            if let Err(e) = configure_static_ipv4("eth0", addr, gateway.as_deref()) {
                eprintln!("[init] Warning: static network configuration failed: {e}");
            }
        }

        setup_dns(&dns_servers);
    } else {
        println!("[init] No static IP configured, trying DHCP...");
        let dhcp_clients = [
            "/sbin/dhclient",
            "/usr/sbin/dhclient",
            "/sbin/udhcpc",
            "/bin/udhcpc",
        ];
        let mut found = false;
        for d in &dhcp_clients {
            if std::path::Path::new(d).exists() {
                match Command::new(d).arg("eth0").output() {
                    Ok(output) if !output.status.success() => {
                        eprintln!(
                            "[init] Warning: DHCP client {d} failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        );
                    }
                    Err(e) => {
                        eprintln!("[init] Warning: Failed to run DHCP client {d}: {e}");
                    }
                    _ => {}
                }
                found = true;
                break;
            }
        }
        if !found {
            setup_dns(&dns_servers);
        }
    }

    // Always ensure DNS
    if !std::path::Path::new("/etc/resolv.conf").exists() {
        setup_dns(&dns_servers);
    }

    Ok(())
}

/// Run an `ip` command best-effort. Returns true if the command succeeded.
fn run_ip(ip_cmd: &Option<String>, args: &[&str]) -> bool {
    if let Some(ref ip) = ip_cmd {
        match Command::new(ip).args(args).output() {
            Ok(output) if !output.status.success() => {
                eprintln!(
                    "[init] Warning: ip {} failed: {}",
                    args.join(" "),
                    String::from_utf8_lossy(&output.stderr)
                );
                false
            }
            Err(e) => {
                eprintln!("[init] Warning: Failed to run ip {}: {e}", args.join(" "));
                false
            }
            _ => true,
        }
    } else {
        false
    }
}

/// Bring up a network interface using ioctl, no external binaries needed.
fn bring_up_interface(name: &str) {
    unsafe {
        let sock = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if sock < 0 {
            return;
        }

        let mut ifr = new_ifreq(name);

        // Get current flags
        if libc::ioctl(sock, libc::SIOCGIFFLAGS as _, &mut ifr) == 0 {
            // Set IFF_UP
            ifr.ifr_ifru.ifru_flags |= libc::IFF_UP as i16;
            libc::ioctl(sock, libc::SIOCSIFFLAGS as _, &ifr);
        }

        libc::close(sock);
    }
}

fn bring_up_loopback() {
    bring_up_interface("lo");
}

fn configure_static_ipv4(
    ifname: &str,
    cidr: &str,
    gateway: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (ip, netmask) = parse_ipv4_cidr(cidr)?;

    unsafe {
        let sock = libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0);
        if sock < 0 {
            return Err(std::io::Error::last_os_error().into());
        }

        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            set_ifreq_sockaddr(sock, ifname, libc::SIOCSIFADDR as _, ip)?;
            let configured_ip = get_ifreq_sockaddr(sock, ifname, libc::SIOCGIFADDR as _)?;
            if configured_ip != ip {
                return Err(format!(
                    "configured IPv4 mismatch on {ifname}: expected {ip}, got {configured_ip}"
                )
                .into());
            }

            set_ifreq_sockaddr(sock, ifname, libc::SIOCSIFNETMASK as _, netmask)?;

            if let Some(gw) = gateway {
                add_default_route(sock, ifname, gw)?;
            }
            Ok(())
        })();

        libc::close(sock);
        result
    }
}

fn parse_ipv4_cidr(cidr: &str) -> Result<(Ipv4Addr, Ipv4Addr), Box<dyn std::error::Error>> {
    let (ip, prefix) = cidr
        .split_once('/')
        .ok_or_else(|| format!("invalid CIDR address: {cidr}"))?;

    let ip: Ipv4Addr = ip.parse()?;
    let prefix_len: u8 = prefix.parse()?;
    if prefix_len > 32 {
        return Err(format!("invalid IPv4 prefix length: {prefix_len}").into());
    }

    let mask = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };

    Ok((ip, Ipv4Addr::from(mask.to_be_bytes())))
}

unsafe fn set_ifreq_sockaddr(
    sock: libc::c_int,
    ifname: &str,
    request: libc::Ioctl,
    addr: Ipv4Addr,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut ifr = new_ifreq(ifname);
    ifr.ifr_ifru.ifru_addr = sockaddr_from_ipv4(addr);

    if libc::ioctl(sock, request, &ifr) != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

unsafe fn get_ifreq_sockaddr(
    sock: libc::c_int,
    ifname: &str,
    request: libc::Ioctl,
) -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
    let mut ifr = new_ifreq(ifname);

    if libc::ioctl(sock, request, &mut ifr) != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    sockaddr_to_ipv4(ifr.ifr_ifru.ifru_addr)
}

unsafe fn add_default_route(
    sock: libc::c_int,
    ifname: &str,
    gateway: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let gateway: Ipv4Addr = gateway.parse()?;
    let ifname = CString::new(ifname)?;

    let mut route: libc::rtentry = std::mem::zeroed();
    route.rt_dst = sockaddr_from_ipv4(Ipv4Addr::UNSPECIFIED);
    route.rt_gateway = sockaddr_from_ipv4(gateway);
    route.rt_genmask = sockaddr_from_ipv4(Ipv4Addr::UNSPECIFIED);
    route.rt_flags = libc::RTF_UP | libc::RTF_GATEWAY;
    route.rt_dev = ifname.as_ptr() as *mut libc::c_char;

    if libc::ioctl(sock, libc::SIOCADDRT as _, &route) != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    Ok(())
}

unsafe fn new_ifreq(ifname: &str) -> libc::ifreq {
    let mut ifr: libc::ifreq = std::mem::zeroed();
    let name_bytes = ifname.as_bytes();
    let copy_len = name_bytes.len().min(libc::IFNAMSIZ - 1);
    std::ptr::copy_nonoverlapping(
        name_bytes.as_ptr(),
        ifr.ifr_name.as_mut_ptr() as *mut u8,
        copy_len,
    );
    ifr
}

fn sockaddr_from_ipv4(addr: Ipv4Addr) -> libc::sockaddr {
    let mut sa: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    sa.sin_family = libc::AF_INET as libc::sa_family_t;
    sa.sin_addr = libc::in_addr {
        s_addr: u32::from_ne_bytes(addr.octets()),
    };

    unsafe { std::mem::transmute::<libc::sockaddr_in, libc::sockaddr>(sa) }
}

fn sockaddr_to_ipv4(sa: libc::sockaddr) -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
    if sa.sa_family != libc::AF_INET as libc::sa_family_t {
        return Err(format!("unexpected sockaddr family: {}", sa.sa_family).into());
    }

    let sin: libc::sockaddr_in = unsafe { std::mem::transmute(sa) };
    Ok(Ipv4Addr::from(sin.sin_addr.s_addr.to_ne_bytes()))
}

fn setup_dns(servers: &[String]) {
    let config = format!(
        "# Generated by CHVM init\n{}",
        servers
            .iter()
            .map(|s| format!("nameserver {s}\n"))
            .collect::<String>()
    );
    match std::fs::write("/etc/resolv.conf", &config) {
        Ok(()) => println!("[init] DNS configured ({})", servers.join(", ")),
        Err(e) => eprintln!("[init] Warning: Failed to configure DNS: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ipv4_cidr_valid() {
        let (ip, netmask) = parse_ipv4_cidr("192.168.200.2/24").unwrap();
        assert_eq!(ip, Ipv4Addr::new(192, 168, 200, 2));
        assert_eq!(netmask, Ipv4Addr::new(255, 255, 255, 0));
    }

    #[test]
    fn parse_ipv4_cidr_rejects_invalid_prefix() {
        assert!(parse_ipv4_cidr("192.168.200.2/33").is_err());
    }

    #[test]
    fn sockaddr_round_trip_ipv4() {
        let addr = Ipv4Addr::new(192, 168, 200, 2);
        let sockaddr = sockaddr_from_ipv4(addr);
        assert_eq!(sockaddr_to_ipv4(sockaddr).unwrap(), addr);
    }
}
