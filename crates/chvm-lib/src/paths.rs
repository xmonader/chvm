use std::env;
use std::path::{Path, PathBuf};

/// Standard directory layout for CHVM data.
#[derive(Debug, Clone)]
pub struct Paths {
    pub base: PathBuf,
    pub images: PathBuf,
    pub vms: PathBuf,
    pub cache: PathBuf,
    pub logs: PathBuf,
    pub kernels: PathBuf,
    pub config_file: PathBuf,
}

impl Paths {
    pub fn new(base: PathBuf) -> Self {
        Self {
            images: base.join("images"),
            vms: base.join("vms"),
            cache: base.join("cache"),
            logs: base.join("logs"),
            kernels: base.join("kernels"),
            config_file: base.join("config.json"),
            base,
        }
    }

    /// Resolve the base directory, respecting CHVM_HOME, SUDO_USER, DOAS_USER.
    pub fn resolve() -> Self {
        let base = if let Ok(home) = env::var("CHVM_HOME") {
            PathBuf::from(home)
        } else if let Ok(sudo_user) = env::var("SUDO_USER") {
            get_user_home(&sudo_user).join(".chvm")
        } else if let Ok(doas_user) = env::var("DOAS_USER") {
            get_user_home(&doas_user).join(".chvm")
        } else {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("/root"))
                .join(".chvm")
        };
        Self::new(base)
    }

    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        for dir in [
            &self.base,
            &self.images,
            &self.vms,
            &self.cache,
            &self.logs,
            &self.kernels,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn vm_dir(&self, vm_id: &str) -> PathBuf {
        self.vms.join(vm_id)
    }

    pub fn image_dir(&self, digest: &str) -> PathBuf {
        self.images.join(digest)
    }
}

fn get_user_home(username: &str) -> PathBuf {
    // Try passwd lookup via nix
    #[cfg(unix)]
    {
        if let Ok(Some(user)) = nix::unistd::User::from_name(username) {
            return user.dir;
        }
    }
    Path::new("/home").join(username)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_paths_new() {
        let base = PathBuf::from("/test/chvm");
        let paths = Paths::new(base.clone());

        assert_eq!(paths.base, PathBuf::from("/test/chvm"));
        assert_eq!(paths.images, PathBuf::from("/test/chvm/images"));
        assert_eq!(paths.vms, PathBuf::from("/test/chvm/vms"));
        assert_eq!(paths.cache, PathBuf::from("/test/chvm/cache"));
        assert_eq!(paths.logs, PathBuf::from("/test/chvm/logs"));
        assert_eq!(paths.kernels, PathBuf::from("/test/chvm/kernels"));
        assert_eq!(paths.config_file, PathBuf::from("/test/chvm/config.json"));
    }

    #[test]
    fn test_vm_dir() {
        let paths = Paths::new(PathBuf::from("/test"));
        let vm_dir = paths.vm_dir("test-vm-123");
        assert_eq!(vm_dir, PathBuf::from("/test/vms/test-vm-123"));
    }

    #[test]
    fn test_image_dir() {
        let paths = Paths::new(PathBuf::from("/test"));
        let image_dir = paths.image_dir("sha256-abc123");
        assert_eq!(image_dir, PathBuf::from("/test/images/sha256-abc123"));
    }

    #[test]
    fn test_ensure_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path().join("chvm");
        let paths = Paths::new(base.clone());

        paths.ensure_dirs().unwrap();

        assert!(paths.base.exists());
        assert!(paths.images.exists());
        assert!(paths.vms.exists());
        assert!(paths.cache.exists());
        assert!(paths.logs.exists());
        assert!(paths.kernels.exists());
    }

    #[test]
    fn test_resolve_with_chvm_home() {
        let temp_dir = TempDir::new().unwrap();
        let custom_home = temp_dir.path().join("custom_chvm");

        env::set_var("CHVM_HOME", custom_home.to_str().unwrap());
        let paths = Paths::resolve();
        env::remove_var("CHVM_HOME");

        assert_eq!(paths.base, custom_home);
    }

    #[test]
    fn test_paths_resolve_creates_valid_structure() {
        let paths = Paths::resolve();

        assert!(paths.base.to_string_lossy().contains(".chvm"));
        assert!(paths.images.ends_with("images"));
        assert!(paths.vms.ends_with("vms"));
        assert!(paths.cache.ends_with("cache"));
        assert!(paths.logs.ends_with("logs"));
        assert!(paths.kernels.ends_with("kernels"));
        assert!(paths.config_file.ends_with("config.json"));
    }

    #[test]
    fn test_get_user_home_fallback() {
        let home = get_user_home("testuser");
        assert_eq!(home, PathBuf::from("/home/testuser"));
    }
}
