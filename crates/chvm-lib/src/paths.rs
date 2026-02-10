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
        for dir in [&self.base, &self.images, &self.vms, &self.cache, &self.logs, &self.kernels] {
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
