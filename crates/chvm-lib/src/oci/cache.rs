use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::error::{ChvmError, Result};
use crate::oci::image_config::ImageConfig;
use crate::oci::layer;
use crate::oci::pull::PullResult;
use crate::oci::registry::sanitize_digest;
use crate::storage::image::inject_runtime_binaries;
use crate::utils::{atomic_write, FileLock};

const PREPARED_ROOTFS_VERSION: &str = "v1";

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
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub prepared_rootfs_paths: HashMap<String, String>,
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

        if let Some(existing) = self.index.images.get_mut(&digest) {
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
            prepared_rootfs_paths: HashMap::new(),
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

    /// Ensure a prepared rootfs exists for a specific runtime variant.
    pub fn ensure_prepared_rootfs(
        &mut self,
        cached: &CachedImage,
        inject_mycelium: bool,
    ) -> Result<String> {
        let variant = prepared_rootfs_variant(inject_mycelium);
        if let Some(path) = cached.prepared_rootfs_paths.get(&variant) {
            if Path::new(path).exists() {
                return Ok(path.clone());
            }
        }

        let digest = sanitize_digest(&cached.digest);
        let rootfs = self.ensure_rootfs(cached)?;
        let digest_dir = self.cache_dir.join(&digest);
        std::fs::create_dir_all(&digest_dir)?;
        let prepared_dir = digest_dir.join(&variant);
        let prepare_lock_path = digest_dir.join(format!("{variant}.lock"));
        let _prepare_lock = FileLock::exclusive(&prepare_lock_path)?;

        if prepared_dir.exists() {
            self.record_prepared_rootfs_path(&digest, &variant, &prepared_dir)?;
            return Ok(prepared_dir.to_string_lossy().to_string());
        }

        let temp_dir = self
            .cache_dir
            .join(&digest)
            .join(format!(".{variant}.tmp-{}", std::process::id()));

        let _ = std::fs::remove_dir_all(&temp_dir);
        std::fs::create_dir_all(&temp_dir)?;

        copy_dir_contents(Path::new(&rootfs), &temp_dir).map_err(|e| {
            ChvmError::OciError(format!(
                "failed to copy rootfs into prepared cache {variant}: {e}"
            ))
        })?;
        inject_runtime_binaries(&temp_dir, inject_mycelium);

        std::fs::rename(&temp_dir, &prepared_dir)?;
        self.record_prepared_rootfs_path(&digest, &variant, &prepared_dir)?;

        Ok(prepared_dir.to_string_lossy().to_string())
    }

    fn record_prepared_rootfs_path(
        &mut self,
        digest: &str,
        variant: &str,
        prepared_dir: &Path,
    ) -> Result<()> {

        let _lock = FileLock::exclusive(&self.index_path())?;
        self.reload_index()?;

        if let Some(img) = self.index.images.get_mut(digest) {
            img.prepared_rootfs_paths
                .insert(variant.to_string(), prepared_dir.to_string_lossy().to_string());
            img.last_used_at = Utc::now();
        }
        self.save_index()?;
        Ok(())
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

fn prepared_rootfs_variant(inject_mycelium: bool) -> String {
    let flavor = if inject_mycelium { "mycelium" } else { "base" };
    format!("prepared-rootfs-{flavor}-{PREPARED_ROOTFS_VERSION}")
}

fn copy_dir_contents(src: &Path, dest: &Path) -> std::io::Result<()> {
    let output = Command::new("cp")
        .arg("-a")
        .arg(format!("{}/.", src.display()))
        .arg(format!("{}/", dest.display()))
        .output()?;

    if output.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!(
            "cp failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_cached_image(cache_dir: &Path) -> CachedImage {
        let digest = "sha256:test";
        let rootfs = cache_dir.join(sanitize_digest(digest)).join("rootfs");
        std::fs::create_dir_all(rootfs.join("bin")).unwrap();
        std::fs::write(rootfs.join("bin/sh"), b"#!/bin/sh\n").unwrap();

        CachedImage {
            digest: digest.into(),
            refs: vec!["docker.io/library/test:latest".into()],
            size: 0,
            created_at: Utc::now(),
            last_used_at: Utc::now(),
            layer_paths: vec![],
            rootfs_path: Some(rootfs.to_string_lossy().to_string()),
            config: None,
            labels: HashMap::new(),
            prepared_rootfs_paths: HashMap::new(),
        }
    }

    #[test]
    fn ensure_prepared_rootfs_persists_variant_path() {
        let temp = tempfile::tempdir().unwrap();
        let mut cache = ImageCache::new(temp.path()).unwrap();
        let cached = make_cached_image(temp.path());
        let digest = sanitize_digest(&cached.digest);
        cache.index.images.insert(digest, cached.clone());
        cache.save_index().unwrap();

        let prepared = cache.ensure_prepared_rootfs(&cached, false).unwrap();
        assert!(Path::new(&prepared).exists());

        let reloaded = cache
            .get(&cached.digest)
            .unwrap()
            .prepared_rootfs_paths
            .get(&prepared_rootfs_variant(false))
            .cloned();
        assert_eq!(reloaded.as_deref(), Some(prepared.as_str()));
    }
}
