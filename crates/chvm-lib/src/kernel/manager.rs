use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use tracing::info;

use crate::error::{ChvmError, Result};

/// Kernel capability information.
#[derive(Debug, Clone)]
pub struct KernelCapabilities {
    pub has_ipv6: bool,
    pub has_tun: bool,
    pub has_virtiofs: bool,
    pub has_virtio_block: bool,
    pub has_vsock: bool,
}

/// Information about a kernel file.
#[derive(Debug, Clone)]
pub struct KernelInfo {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub is_symlink: bool,
    pub symlink_target: Option<String>,
    pub capabilities: Option<KernelCapabilities>,
}

/// Manages kernel selection, downloading, and extraction.
pub struct KernelManager {
    kernel_dir: PathBuf,
}

impl KernelManager {
    pub fn new(kernel_dir: &Path) -> Self {
        Self {
            kernel_dir: kernel_dir.to_path_buf(),
        }
    }

    /// Select the best kernel for the given network mode.
    pub fn select_kernel(&self, network_mode: &str) -> Result<String> {
        let kernels = self.list_kernels()?;
        if kernels.is_empty() {
            return Err(ChvmError::KernelNotFound(format!(
                "no kernels found in {}. Run 'chvm setup all' or 'chvm kernel extract-host'",
                self.kernel_dir.display()
            )));
        }

        // For mycelium, prefer kernel with IPv6 + TUN
        if network_mode == "mycelium" {
            let default = self.kernel_dir.join("vmlinux");
            if default.exists() && self.kernel_supports_mycelium(&default) {
                return Ok(default.to_string_lossy().to_string());
            }

            let mycelium_kernel = self.kernel_dir.join("vmlinux-mycelium");
            if mycelium_kernel.exists() {
                return Ok(mycelium_kernel.to_string_lossy().to_string());
            }

            let host_kernel = self.kernel_dir.join("vmlinux-host");
            if host_kernel.exists() {
                return Ok(host_kernel.to_string_lossy().to_string());
            }

            // Try to extract host kernel
            if let Ok(path) = self.extract_host_kernel() {
                return Ok(path);
            }
        }

        // Default: vmlinux symlink or first available
        let default = self.kernel_dir.join("vmlinux");
        if default.exists() {
            return Ok(default.to_string_lossy().to_string());
        }

        kernels
            .first()
            .map(|k| k.path.to_string_lossy().to_string())
            .ok_or_else(|| ChvmError::KernelNotFound("no suitable kernel found".into()))
    }

    /// List all available kernels.
    pub fn list_kernels(&self) -> Result<Vec<KernelInfo>> {
        let entries = match std::fs::read_dir(&self.kernel_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };

        let mut kernels = Vec::new();
        for entry in entries.flatten() {
            // Use file_type() from DirEntry which does lstat (detects symlinks correctly)
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                continue;
            }

            let path = entry.path();
            let is_symlink = ft.is_symlink();

            // Use symlink_metadata to get size without following symlinks,
            // but for size reporting follow the link to get actual file size
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

            kernels.push(KernelInfo {
                name: entry.file_name().to_string_lossy().to_string(),
                size,
                is_symlink,
                symlink_target: if is_symlink {
                    std::fs::read_link(&path)
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                } else {
                    None
                },
                capabilities: None,
                path,
            });
        }

        Ok(kernels)
    }

    /// Set the default kernel by creating a symlink.
    pub fn set_default(&self, kernel_name: &str) -> Result<()> {
        let kernel_path = self.kernel_dir.join(kernel_name);
        if !kernel_path.exists() {
            return Err(ChvmError::KernelNotFound(kernel_name.to_string()));
        }

        let symlink_path = self.kernel_dir.join("vmlinux");
        let _ = std::fs::remove_file(&symlink_path);
        std::os::unix::fs::symlink(kernel_name, &symlink_path)?;
        Ok(())
    }

    /// Extract vmlinux from the running host's kernel.
    pub fn extract_host_kernel(&self) -> Result<String> {
        let uname = Command::new("uname").arg("-r").output()?;
        let version = String::from_utf8_lossy(&uname.stdout).trim().to_string();

        let possible_paths = [
            format!("/boot/vmlinuz-{version}"),
            format!("/boot/vmlinux-{version}"),
            "/boot/vmlinuz".into(),
            "/boot/vmlinux".into(),
        ];

        let kernel_image = possible_paths
            .iter()
            .find(|p| Path::new(p).exists())
            .ok_or_else(|| {
                ChvmError::KernelNotFound(format!(
                    "could not find host kernel image (tried /boot/vmlinuz-{version})"
                ))
            })?;

        info!(source = %kernel_image, "Extracting host kernel");

        std::fs::create_dir_all(&self.kernel_dir)?;
        let dest = self.kernel_dir.join("vmlinux-host");

        extract_vmlinux(kernel_image, &dest)?;

        // Verify
        let meta = std::fs::metadata(&dest)?;
        if meta.len() < 1_000_000 {
            let _ = std::fs::remove_file(&dest);
            return Err(ChvmError::KernelNotFound(format!(
                "extracted kernel too small ({} bytes)",
                meta.len()
            )));
        }

        Ok(dest.to_string_lossy().to_string())
    }

    /// Detect kernel capabilities by checking embedded config.
    pub fn detect_capabilities(&self, kernel_path: &Path) -> Option<KernelCapabilities> {
        let config = extract_kernel_config(kernel_path)?;

        Some(KernelCapabilities {
            has_ipv6: config
                .get("CONFIG_IPV6")
                .map(|v| v == "y" || v == "m")
                .unwrap_or(false),
            has_tun: config
                .get("CONFIG_TUN")
                .map(|v| v == "y" || v == "m")
                .unwrap_or(false),
            has_virtiofs: config
                .get("CONFIG_VIRTIO_FS")
                .map(|v| v == "y" || v == "m")
                .unwrap_or(false),
            has_virtio_block: config
                .get("CONFIG_VIRTIO_BLK")
                .map(|v| v == "y" || v == "m")
                .unwrap_or(false),
            has_vsock: config
                .get("CONFIG_VSOCKETS")
                .map(|v| v == "y" || v == "m")
                .unwrap_or(false),
        })
    }

    /// Check mycelium support.
    pub fn check_mycelium_support(&self) -> (bool, String) {
        let default = self.kernel_dir.join("vmlinux");
        if !default.exists() {
            return (
                false,
                "No default kernel found. Run 'chvm setup all' first.".into(),
            );
        }

        let host_kernel = self.kernel_dir.join("vmlinux-host");
        if host_kernel.exists() {
            return (true, "Host kernel available for mycelium support.".into());
        }

        if let Some(caps) = self.detect_capabilities(&default) {
            if caps.has_ipv6 && caps.has_tun {
                return (true, "Default kernel supports mycelium.".into());
            }
            let mut missing = Vec::new();
            if !caps.has_ipv6 {
                missing.push("IPv6 (CONFIG_IPV6)");
            }
            if !caps.has_tun {
                missing.push("TUN (CONFIG_TUN)");
            }
            return (
                false,
                format!(
                    "Default kernel missing: {}. Run 'chvm kernel extract-host'.",
                    missing.join(", ")
                ),
            );
        }

        (
            false,
            "Cannot verify kernel capabilities. If mycelium fails, run 'chvm kernel extract-host'."
                .into(),
        )
    }

    fn kernel_supports_mycelium(&self, path: &Path) -> bool {
        if let Some(caps) = self.detect_capabilities(path) {
            return caps.has_ipv6 && caps.has_tun;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains("host") {
            return true;
        }
        if name.contains("firecracker") {
            return false;
        }
        true // Assume it works
    }
}

fn extract_kernel_config(kernel_path: &Path) -> Option<HashMap<String, String>> {
    // Try /proc/config.gz first
    if let Ok(config) = read_proc_config() {
        return Some(parse_kernel_config(&config));
    }

    // Try embedded IKCONFIG
    let data = std::fs::read(kernel_path).ok()?;
    let magic_start = b"IKCFG_ST";
    let magic_end = b"IKCFG_ED";

    let start = data
        .windows(magic_start.len())
        .position(|w| w == magic_start)?
        + magic_start.len();

    let end = data[start..]
        .windows(magic_end.len())
        .position(|w| w == magic_end)?
        + start;

    let config_data = &data[start..end];
    let mut decoder = GzDecoder::new(config_data);
    let mut decompressed = String::new();
    decoder.read_to_string(&mut decompressed).ok()?;

    Some(parse_kernel_config(&decompressed))
}

fn read_proc_config() -> std::io::Result<String> {
    let file = std::fs::File::open("/proc/config.gz")?;
    let mut decoder = GzDecoder::new(file);
    let mut data = String::new();
    decoder.read_to_string(&mut data)?;
    Ok(data)
}

fn parse_kernel_config(data: &str) -> HashMap<String, String> {
    let mut config = HashMap::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            config.insert(key.to_string(), value.to_string());
        }
    }
    config
}

fn extract_vmlinux(src: &str, dest: &Path) -> Result<()> {
    let data = std::fs::read(src)?;

    let elf_magic = [0x7f, 0x45, 0x4c, 0x46]; // "\x7fELF"

    // Check if already ELF
    if data.len() > 4 && data[..4] == elf_magic {
        std::fs::write(dest, &data)?;
        return Ok(());
    }

    // Try using the system's extract-vmlinux script (most reliable)
    if let Ok(uname) = Command::new("uname").arg("-r").output() {
        let version = String::from_utf8_lossy(&uname.stdout).trim().to_string();
        let script = format!("/usr/src/linux-headers-{version}/scripts/extract-vmlinux");
        if Path::new(&script).exists() {
            let output = Command::new("bash").arg(&script).arg(src).output()?;
            if output.status.success() && output.stdout.len() > 4 && output.stdout[..4] == elf_magic
            {
                std::fs::write(dest, &output.stdout)?;
                return Ok(());
            }
        }
    }

    // Find gzip compressed ELF
    let gzip_magic = [0x1f, 0x8b];
    for i in 0..data.len().saturating_sub(2) {
        if data[i] == gzip_magic[0] && data[i + 1] == gzip_magic[1] {
            let mut decoder = GzDecoder::new(&data[i..]);
            let mut decompressed = Vec::new();
            if decoder.read_to_end(&mut decompressed).is_ok()
                && decompressed.len() > 4
                && decompressed[..4] == elf_magic
            {
                std::fs::write(dest, &decompressed)?;
                return Ok(());
            }
        }
    }

    // Try finding compressed streams within the bzImage and decompressing them
    let magic_bytes: &[(&str, &[u8])] = &[
        ("zstd", &[0x28, 0xb5, 0x2f, 0xfd]),
        ("lz4", &[0x02, 0x21, 0x4c, 0x18]),
        ("xz", &[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]),
    ];

    for (tool, magic) in magic_bytes {
        for i in 0..data.len().saturating_sub(magic.len()) {
            if data[i..i + magic.len()] == **magic
                && try_decompress_with_tool(&data[i..], dest, tool)?
            {
                return Ok(());
            }
        }
    }

    Err(ChvmError::KernelNotFound(
        "could not extract vmlinux from kernel image".into(),
    ))
}

fn try_decompress_with_tool(data: &[u8], dest: &Path, tool: &str) -> Result<bool> {
    if Command::new("which")
        .arg(tool)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        let tmp = tempfile::NamedTempFile::new()?;
        std::fs::write(tmp.path(), data)?;

        let output = Command::new(tool)
            .args(["-d", "-c"])
            .arg(tmp.path())
            .output()?;

        let elf_magic = [0x7f, 0x45, 0x4c, 0x46];
        if output.status.success() && output.stdout.len() > 4 && output.stdout[..4] == elf_magic {
            std::fs::write(dest, &output.stdout)?;
            return Ok(true);
        }
    }
    Ok(false)
}
