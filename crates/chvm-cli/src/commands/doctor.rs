use crate::cli::DoctorArgs;
use chvm_lib::hypervisor::process::find_ch_binary;
use chvm_lib::paths::Paths;
use std::process::Command;

pub fn execute(_args: &DoctorArgs) -> anyhow::Result<()> {
    println!("CHVM Doctor - System Check\n");

    let mut all_ok = true;

    // Check Cloud Hypervisor
    print!("Cloud Hypervisor: ");
    match find_ch_binary() {
        Some(path) => {
            let version = Command::new(&path).arg("--version").output()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_else(|_| "unknown version".into());
            println!("OK ({path}) - {version}");
        }
        None => {
            println!("NOT FOUND");
            println!("  Install: https://github.com/cloud-hypervisor/cloud-hypervisor/releases");
            all_ok = false;
        }
    }

    // Check virtiofsd
    print!("virtiofsd: ");
    let virtiofsd_paths = [
        "virtiofsd",
        "/usr/libexec/virtiofsd",
        "/usr/lib/virtiofsd",
        "/usr/local/bin/virtiofsd",
    ];
    let found = virtiofsd_paths.iter().any(|p| {
        which::which(p).is_ok() || std::path::Path::new(p).exists()
    });
    if found {
        println!("OK");
    } else {
        println!("NOT FOUND (optional, needed for virtiofs storage)");
    }

    // Check kernel
    let paths = Paths::resolve();
    print!("Kernel: ");
    let default_kernel = paths.kernels.join("vmlinux");
    if default_kernel.exists() {
        let size = std::fs::metadata(&default_kernel)
            .map(|m| format!("{}MB", m.len() / 1_048_576))
            .unwrap_or_else(|_| "?".into());
        println!("OK ({}) - {size}", default_kernel.display());
    } else {
        let kernel_count = std::fs::read_dir(&paths.kernels)
            .map(|e| e.count())
            .unwrap_or(0);
        if kernel_count > 0 {
            println!("WARN - No default kernel symlink, but {} kernels found", kernel_count);
        } else {
            println!("NOT FOUND");
            println!("  Run: chvm kernel extract-host");
            all_ok = false;
        }
    }

    // Check mkfs.ext4
    print!("mkfs.ext4: ");
    if which::which("mkfs.ext4").is_ok() {
        println!("OK");
    } else {
        println!("NOT FOUND (needed for block storage)");
        all_ok = false;
    }

    // Check iptables
    print!("iptables: ");
    if which::which("iptables").is_ok() {
        println!("OK");
    } else {
        println!("NOT FOUND (needed for TAP networking)");
    }

    // Check ip command
    print!("ip command: ");
    if which::which("ip").is_ok() {
        println!("OK");
    } else {
        println!("NOT FOUND (needed for networking)");
        all_ok = false;
    }

    // Check /dev/kvm
    print!("/dev/kvm: ");
    if std::path::Path::new("/dev/kvm").exists() {
        println!("OK");
    } else {
        println!("NOT FOUND");
        println!("  Enable KVM: modprobe kvm_intel (or kvm_amd)");
        all_ok = false;
    }

    // Check data directory
    print!("Data directory: ");
    if paths.base.exists() {
        println!("OK ({})", paths.base.display());
    } else {
        println!("Will be created at {}", paths.base.display());
    }

    println!();
    if all_ok {
        println!("All checks passed! CHVM is ready to use.");
    } else {
        println!("Some checks failed. Please install missing dependencies.");
    }

    Ok(())
}
