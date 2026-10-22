pub mod attach;
pub mod console;
pub mod create;
pub mod doctor;
pub mod exec;
pub mod images;
pub mod inspect;
pub mod kernel;
pub mod list;
pub mod login;
pub mod logout;
pub mod logs;
pub mod prune;
pub mod pull;
pub mod rm;
pub mod run;
pub mod setup;
pub mod start;
pub mod stats;
pub mod stop;
pub mod version;

use chvm_lib::oci::{registry_from_image_ref, resolve_credentials};
use chvm_lib::vm::manager::CreateOptions;
use chvm_lib::vm::state::{MountConfig, NetworkMode, PortForward};

/// Common fields shared between RunArgs and CreateArgs.
pub trait CommonVmArgs {
    fn name(&self) -> Option<String>;
    fn image(&self) -> &str;
    fn rootfs(&self) -> Option<String>;
    fn cpus(&self) -> u32;
    fn memory(&self) -> u32;
    fn storage(&self) -> Option<&str>;
    fn disk_size(&self) -> Option<&str>;
    fn storage_quota(&self) -> Option<&str>;
    fn network(&self) -> &str;
    fn publish(&self) -> &[String];
    fn env(&self) -> &[String];
    fn volume(&self) -> &[String];
    fn kernel(&self) -> Option<String>;
    fn mycelium_peers(&self) -> &[String];
    fn cmd(&self) -> &[String];
}

impl CommonVmArgs for crate::cli::RunArgs {
    fn name(&self) -> Option<String> {
        self.name.clone()
    }
    fn image(&self) -> &str {
        &self.image
    }
    fn rootfs(&self) -> Option<String> {
        self.rootfs.clone()
    }
    fn cpus(&self) -> u32 {
        self.cpus
    }
    fn memory(&self) -> u32 {
        self.memory
    }
    fn storage(&self) -> Option<&str> {
        self.storage.as_deref()
    }
    fn disk_size(&self) -> Option<&str> {
        self.disk_size.as_deref()
    }
    fn storage_quota(&self) -> Option<&str> {
        self.storage_quota.as_deref()
    }
    fn network(&self) -> &str {
        &self.network
    }
    fn publish(&self) -> &[String] {
        &self.publish
    }
    fn env(&self) -> &[String] {
        &self.env
    }
    fn volume(&self) -> &[String] {
        &self.volume
    }
    fn kernel(&self) -> Option<String> {
        self.kernel.clone()
    }
    fn mycelium_peers(&self) -> &[String] {
        &self.mycelium_peers
    }
    fn cmd(&self) -> &[String] {
        &self.cmd
    }
}

impl CommonVmArgs for crate::cli::CreateArgs {
    fn name(&self) -> Option<String> {
        self.name.clone()
    }
    fn image(&self) -> &str {
        &self.image
    }
    fn rootfs(&self) -> Option<String> {
        self.rootfs.clone()
    }
    fn cpus(&self) -> u32 {
        self.cpus
    }
    fn memory(&self) -> u32 {
        self.memory
    }
    fn storage(&self) -> Option<&str> {
        self.storage.as_deref()
    }
    fn disk_size(&self) -> Option<&str> {
        self.disk_size.as_deref()
    }
    fn storage_quota(&self) -> Option<&str> {
        self.storage_quota.as_deref()
    }
    fn network(&self) -> &str {
        &self.network
    }
    fn publish(&self) -> &[String] {
        &self.publish
    }
    fn env(&self) -> &[String] {
        &self.env
    }
    fn volume(&self) -> &[String] {
        &self.volume
    }
    fn kernel(&self) -> Option<String> {
        self.kernel.clone()
    }
    fn mycelium_peers(&self) -> &[String] {
        &self.mycelium_peers
    }
    fn cmd(&self) -> &[String] {
        &self.cmd
    }
}

/// Truncate a string to max length, adding "..." if truncated.
/// Uses char boundaries to avoid splitting multi-byte UTF-8 characters.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    if max <= 3 {
        return ".".repeat(max);
    }
    // Find the byte index of the (max - 3)th character
    let keep = max - 3;
    let end = s
        .char_indices()
        .nth(keep)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    format!("{}...", &s[..end])
}

/// Build CreateOptions from any args implementing CommonVmArgs.
pub fn build_create_options(
    args: &dyn CommonVmArgs,
    detach: bool,
    remove_after: bool,
) -> anyhow::Result<CreateOptions> {
    let block_disk_size_bytes = args
        .disk_size()
        .map(parse_size_bytes)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;
    let writable_quota_bytes = args
        .storage_quota()
        .map(parse_size_bytes)
        .transpose()
        .map_err(|e| anyhow::anyhow!(e))?;

    let port_forwards: Vec<PortForward> = args
        .publish()
        .iter()
        .map(|p| p.parse())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    let mounts: Vec<MountConfig> = args
        .volume()
        .iter()
        .map(|v| v.parse())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e: String| anyhow::anyhow!(e))?;

    let registry = registry_from_image_ref(args.image());
    let credentials = resolve_credentials(&registry);
    let network_mode = args
        .network()
        .parse::<NetworkMode>()
        .map_err(|e| anyhow::anyhow!("invalid network mode '{}': {e}", args.network()))?;
    let storage_backend = match args.storage() {
        Some("block" | "virtiofs") => args.storage().map(str::to_string),
        Some(other) => {
            return Err(anyhow::anyhow!(
                "invalid storage backend '{}': expected block or virtiofs",
                other
            ));
        }
        None => None,
    };

    Ok(CreateOptions {
        name: args.name(),
        image_ref: if args.rootfs().is_some() {
            None
        } else {
            Some(args.image().to_string())
        },
        rootfs_path: args.rootfs(),
        vcpu: Some(args.cpus()),
        memory_mb: Some(args.memory()),
        storage_backend,
        block_disk_size_bytes,
        writable_quota_bytes,
        network_mode: Some(network_mode),
        port_forwards,
        env: args.env().to_vec(),
        cmd: args.cmd().to_vec(),
        mounts,
        kernel_path: args.kernel(),
        detach,
        remove_after,
        mycelium_peers: args.mycelium_peers().to_vec(),
        credentials: Some(credentials),
    })
}

fn parse_size_bytes(raw: &str) -> std::result::Result<u64, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Err("size cannot be empty".into());
    }

    let split_at = value
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, suffix) = value.split_at(split_at);
    if number.is_empty() {
        return Err(format!("invalid size: {raw}"));
    }

    let base: u64 = number
        .parse()
        .map_err(|_| format!("invalid size number: {raw}"))?;

    let multiplier = match suffix.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "ki" | "kib" => 1024,
        "m" | "mb" | "mi" | "mib" => 1024 * 1024,
        "g" | "gb" | "gi" | "gib" => 1024 * 1024 * 1024,
        "t" | "tb" | "ti" | "tib" => 1024_u64.pow(4),
        _ => return Err(format!("unsupported size suffix in {raw}")),
    };

    base.checked_mul(multiplier)
        .ok_or_else(|| format!("size is too large: {raw}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeArgs {
        storage: Option<String>,
        disk_size: Option<String>,
        storage_quota: Option<String>,
    }

    impl CommonVmArgs for FakeArgs {
        fn name(&self) -> Option<String> {
            Some("test-vm".to_string())
        }
        fn image(&self) -> &str {
            "ubuntu:24.04"
        }
        fn rootfs(&self) -> Option<String> {
            None
        }
        fn cpus(&self) -> u32 {
            1
        }
        fn memory(&self) -> u32 {
            512
        }
        fn storage(&self) -> Option<&str> {
            self.storage.as_deref()
        }
        fn disk_size(&self) -> Option<&str> {
            self.disk_size.as_deref()
        }
        fn storage_quota(&self) -> Option<&str> {
            self.storage_quota.as_deref()
        }
        fn network(&self) -> &str {
            "mycelium"
        }
        fn publish(&self) -> &[String] {
            &[]
        }
        fn env(&self) -> &[String] {
            &[]
        }
        fn volume(&self) -> &[String] {
            &[]
        }
        fn kernel(&self) -> Option<String> {
            None
        }
        fn mycelium_peers(&self) -> &[String] {
            &[]
        }
        fn cmd(&self) -> &[String] {
            &[]
        }
    }

    #[test]
    fn build_create_options_leaves_storage_unset_when_not_overridden() {
        let args = FakeArgs {
            storage: None,
            disk_size: None,
            storage_quota: None,
        };
        let opts = build_create_options(&args, false, false).unwrap();
        assert_eq!(opts.storage_backend, None);
        assert_eq!(opts.block_disk_size_bytes, None);
        assert_eq!(opts.writable_quota_bytes, None);
    }

    #[test]
    fn build_create_options_preserves_storage_override() {
        let args = FakeArgs {
            storage: Some("block".to_string()),
            disk_size: None,
            storage_quota: None,
        };
        let opts = build_create_options(&args, false, false).unwrap();
        assert_eq!(opts.storage_backend.as_deref(), Some("block"));
    }

    #[test]
    fn build_create_options_parses_size_flags() {
        let args = FakeArgs {
            storage: Some("virtiofs".to_string()),
            disk_size: Some("2G".to_string()),
            storage_quota: Some("1536M".to_string()),
        };
        let opts = build_create_options(&args, false, false).unwrap();
        assert_eq!(opts.block_disk_size_bytes, Some(2 * 1024 * 1024 * 1024));
        assert_eq!(opts.writable_quota_bytes, Some(1536 * 1024 * 1024));
    }

    #[test]
    fn parse_size_bytes_accepts_common_suffixes() {
        assert_eq!(parse_size_bytes("512").unwrap(), 512);
        assert_eq!(parse_size_bytes("1K").unwrap(), 1024);
        assert_eq!(parse_size_bytes("2MiB").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size_bytes("3g").unwrap(), 3 * 1024 * 1024 * 1024);
    }

    #[test]
    fn parse_size_bytes_rejects_unknown_suffix() {
        assert!(parse_size_bytes("10P").is_err());
    }
}
