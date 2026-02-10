use std::path::{Path, PathBuf};

use futures::StreamExt;
use oci_distribution::client::{ClientConfig, ClientProtocol};
use oci_distribution::secrets::RegistryAuth;
use oci_distribution::{Client, Reference};
use tokio::io::AsyncWriteExt;
use tracing::info;

use crate::oci::image_config::ImageConfig;
use crate::oci::registry::sanitize_digest;

/// Result of pulling an OCI image.
pub struct PullResult {
    pub reference: String,
    pub digest: String,
    pub config: Option<ImageConfig>,
    pub layer_paths: Vec<String>,
    pub cache_dir: PathBuf,
}

/// Pull an OCI image from a registry.
pub async fn pull_image(image_ref: &str, images_dir: &Path) -> anyhow::Result<PullResult> {
    let normalized = crate::oci::registry::normalize_reference(image_ref);
    info!(image = %normalized, "Pulling image");

    let reference: Reference = normalized.parse()
        .map_err(|e| anyhow::anyhow!("invalid image reference: {e}"))?;

    let client_config = ClientConfig {
        protocol: ClientProtocol::Https,
        platform_resolver: Some(Box::new(|manifests| {
            // Select linux/amd64 platform
            manifests.iter().find(|m| {
                m.platform.as_ref().is_some_and(|p| {
                    p.os == "linux" && p.architecture == "amd64"
                })
            }).map(|m| m.digest.clone())
        })),
        ..Default::default()
    };

    let client = Client::new(client_config);
    let auth = RegistryAuth::Anonymous;

    // Use pull_manifest_and_config which handles ImageIndex resolution via platform_resolver
    let (img_manifest, digest_str, config_str) = client.pull_manifest_and_config(&reference, &auth).await?;

    let safe_digest = sanitize_digest(&digest_str);
    let image_dir = images_dir.join(&safe_digest);
    std::fs::create_dir_all(&image_dir)?;

    info!(digest = %digest_str, "Got manifest");

    // Parse config for image config
    let config = match serde_json::from_str::<serde_json::Value>(&config_str) {
        Ok(val) => ImageConfig::from_oci_config(&val),
        Err(_) => None,
    };

    // Pull layers
    let mut layer_paths = Vec::new();
    for (i, layer) in img_manifest.layers.iter().enumerate() {
        let layer_digest = sanitize_digest(&layer.digest);
        let layer_path = image_dir.join(format!("layer-{}-{}.tar.gz", i, layer_digest));

        if layer_path.exists() {
            info!(layer = i, "Layer already cached");
            layer_paths.push(layer_path.to_string_lossy().to_string());
            continue;
        }

        info!(layer = i + 1, total = img_manifest.layers.len(), "Downloading layer");

        // Stream layer directly to disk to avoid buffering large layers in memory
        let mut stream = client.pull_blob_stream(&reference, layer).await?;
        let mut file = tokio::fs::File::create(&layer_path).await?;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
        drop(file);
        layer_paths.push(layer_path.to_string_lossy().to_string());
    }

    Ok(PullResult {
        reference: normalized,
        digest: digest_str,
        config,
        layer_paths,
        cache_dir: image_dir,
    })
}
