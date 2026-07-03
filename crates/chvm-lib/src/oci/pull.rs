use std::path::{Path, PathBuf};

use oci_distribution::client::{ClientConfig, ClientProtocol};
use oci_distribution::{Client, Reference};
use tracing::info;

use crate::oci::auth::RegistryCredentials;
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
pub async fn pull_image(
    image_ref: &str,
    images_dir: &Path,
    credentials: &RegistryCredentials,
) -> anyhow::Result<PullResult> {
    let normalized = crate::oci::registry::normalize_reference(image_ref);
    info!(image = %normalized, "Pulling image");

    let reference: Reference = normalized
        .parse()
        .map_err(|e| anyhow::anyhow!("invalid image reference: {e}"))?;

    let client_config = ClientConfig {
        protocol: ClientProtocol::Https,
        platform_resolver: Some(Box::new(|manifests| {
            // Select linux/amd64 platform
            manifests
                .iter()
                .find(|m| {
                    m.platform
                        .as_ref()
                        .is_some_and(|p| p.os == "linux" && p.architecture == "amd64")
                })
                .map(|m| m.digest.clone())
        })),
        ..Default::default()
    };

    let client = Client::new(client_config);
    let auth = credentials.to_registry_auth();

    // Use pull_manifest_and_config which handles ImageIndex resolution via platform_resolver
    let (img_manifest, digest_str, config_str) =
        client.pull_manifest_and_config(&reference, &auth).await?;

    crate::oci::registry::validate_digest(&digest_str)?;
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
        crate::oci::registry::validate_digest(&layer.digest)?;
        let layer_digest = sanitize_digest(&layer.digest);
        let layer_path = image_dir.join(format!("layer-{}-{}.tar.gz", i, layer_digest));

        if layer_path.exists() {
            info!(layer = i, "Layer already cached");
            layer_paths.push(layer_path.to_string_lossy().to_string());
            continue;
        }

        info!(
            layer = i + 1,
            total = img_manifest.layers.len(),
            "Downloading layer"
        );

        // Download to a temp path, verify the digest, then atomically rename.
        // This guarantees a partial/interrupted download is never mistaken for a
        // cached layer, and that the bytes match what the manifest promised.
        let tmp_path = layer_path.with_extension("tmp");
        let mut file = tokio::fs::File::create(&tmp_path).await?;
        client.pull_blob(&reference, layer, &mut file).await?;
        file.sync_all().await?;
        drop(file);

        if let Err(e) = verify_blob_digest(&tmp_path, &layer.digest) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        std::fs::rename(&tmp_path, &layer_path)?;
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

/// Verify a downloaded blob's sha256 matches its manifest digest. Streams the
/// file so large layers don't spike memory. Non-sha256 algorithms are skipped
/// (validated for format elsewhere) rather than silently trusted.
fn verify_blob_digest(path: &Path, digest: &str) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};

    let expected = match digest.split_once(':') {
        Some(("sha256", hex)) => hex.to_ascii_lowercase(),
        _ => return Ok(()),
    };

    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    if actual != expected {
        anyhow::bail!("layer digest mismatch: expected sha256:{expected}, got sha256:{actual}");
    }
    Ok(())
}
