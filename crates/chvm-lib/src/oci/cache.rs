use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{ChvmError, Result};
use crate::oci::image_config::ImageConfig;
use crate::oci::layer;
use crate::oci::pull::PullResult;
use crate::oci::registry::sanitize_digest;
use crate::utils::{atomic_write, FileLock};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheIndex {
    pub images: HashMap<String, CachedImage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedImage {
    pub digest: String,
    pub refs: Vec<String>,
    pub size: u64,
    pub created_at: DateTime<Utc>,
    pub last_used_at: DateTime<Utc>,
    pub layer_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rootfs_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<ImageConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub labels: HashMap<String, String>,
}

pub struct ImageCache {
    cache_dir: PathBuf,
    index: CacheIndex,
}

impl ImageCache {
    pub fn new(cache_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(cache_dir)?;
        let index_path = cache_dir.join("index.json");
        let index = if index_path.exists() {
            load_index(cache_dir)?
        } else {
            CacheIndex {
                images: HashMap::new(),
            }
        };
        Ok(Self {
            cache_dir: cache_dir.to_path_buf(),
            index,
        })
    }

    fn index_path(&self) -> PathBuf {
        self.cache_dir.join("index.json")
    }

    fn reload_index(&mut self) -> Result<()> {
        let path = self.index_path();
        if !path.exists() {
            self.index = CacheIndex {
                images: HashMap::new(),
            };
            return Ok(());
        }
        self.index = load_index(&self.cache_dir)?;
        Ok(())
    }

    pub fn add(&mut self, result: &PullResult) -> Result<CachedImage> {
        let _lock = FileLock::exclusive(&self.index_path())?;
        self.reload_index()?;

        let digest = sanitize_digest(&result.digest);

        if self.index.images.contains_key(&digest) {
            let existing = self.index.images.get_mut(&digest).unwrap();
            existing.last_used_at = Utc::now();
            if !existing.refs.contains(&result.reference) {
                existing.refs.push(result.reference.clone());
            }
            let cloned = existing.clone();
            self.save_index()?;
            return Ok(cloned);
        }

        let total_size: u64 = result
            .layer_paths
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum();

        let cached = CachedImage {
            digest: result.digest.clone(),
            refs: vec![result.reference.clone()],
            size: total_size,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
            layer_paths: result.layer_paths.clone(),
            rootfs_path: None,
            config: result.config.clone(),
            labels: result
                .config
                .as_ref()
                .map(|c| c.labels.clone())
                .unwrap_or_default(),
        };

        self.index.images.insert(digest, cached.clone());
        self.save_index()?;
        Ok(cached)
    }

    pub fn get(&self, reference: &str) -> Result<CachedImage> {
        // Try by digest
        let digest = sanitize_digest(reference);
        if let Some(img) = self.index.images.get(&digest) {
            return Ok(img.clone());
        }

        // Try by reference
        let normalized = crate::oci::registry::normalize_reference(reference);
        for img in self.index.images.values() {
            if img.refs.contains(&normalized) || img.refs.iter().any(|r| r == reference) {
                return Ok(img.clone());
            }
        }

        Err(ChvmError::ImageNotFound(reference.to_string()))
    }

    pub fn list(&self) -> Vec<&CachedImage> {
        let mut images: Vec<_> = self.index.images.values().collect();
        images.sort_by(|a, b| b.last_used_at.cmp(&a.last_used_at));
        images
    }

    pub fn remove(&mut self, reference: &str) -> Result<()> {
        let _lock = FileLock::exclusive(&self.index_path())?;
        self.reload_index()?;

        let digest = sanitize_digest(reference);
        if let Some(img) = self.index.images.remove(&digest) {
            let image_dir = self.cache_dir.join(&digest);
            let _ = std::fs::remove_dir_all(&image_dir);
            if let Some(ref rootfs) = img.rootfs_path {
                let _ = std::fs::remove_dir_all(rootfs);
            }
            self.save_index()?;
            return Ok(());
        }

        // Try by reference
        let to_remove: Option<String> = self
            .index
            .images
            .iter()
            .find(|(_, img)| img.refs.iter().any(|r| r == reference))
            .map(|(k, _)| k.clone());

        if let Some(key) = to_remove {
            if let Some(img) = self.index.images.remove(&key) {
                let image_dir = self.cache_dir.join(&key);
                let _ = std::fs::remove_dir_all(&image_dir);
                if let Some(ref rootfs) = img.rootfs_path {
                    let _ = std::fs::remove_dir_all(rootfs);
                }
            }
            self.save_index()?;
            Ok(())
        } else {
            Err(ChvmError::ImageNotFound(reference.to_string()))
        }
    }

    /// Ensure rootfs is extracted, returning the path.
    pub fn ensure_rootfs(&mut self, cached: &CachedImage) -> Result<String> {
        if let Some(ref rootfs) = cached.rootfs_path {
            if Path::new(rootfs).exists() {
                return Ok(rootfs.clone());
            }
        }

        let digest = sanitize_digest(&cached.digest);
        let rootfs_dir = self.cache_dir.join(&digest).join("rootfs");

        info!(digest = %cached.digest, dest = %rootfs_dir.display(), "Extracting rootfs");

        layer::extract_layers(&cached.layer_paths, &rootfs_dir)
            .map_err(|e| ChvmError::OciError(format!("failed to extract layers: {e}")))?;

        let _lock = FileLock::exclusive(&self.index_path())?;
        self.reload_index()?;

        if let Some(img) = self.index.images.get_mut(&digest) {
            img.rootfs_path = Some(rootfs_dir.to_string_lossy().to_string());
            img.last_used_at = Utc::now();
        }
        self.save_index()?;

        Ok(rootfs_dir.to_string_lossy().to_string())
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    fn save_index(&self) -> Result<()> {
        let path = self.index_path();
        let data = serde_json::to_string_pretty(&self.index)?;
        atomic_write(&path, data.as_bytes())?;
        Ok(())
    }
}

fn load_index(cache_dir: &Path) -> Result<CacheIndex> {
    let path = cache_dir.join("index.json");
    let data = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&data)?)
}
