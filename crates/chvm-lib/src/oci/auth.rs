use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

/// Credentials for authenticating with an OCI registry.
/// Either anonymous or basic auth (username + password together).
#[derive(Default, Debug, Clone)]
pub enum RegistryCredentials {
    #[default]
    Anonymous,
    Basic {
        username: String,
        password: String,
    },
}

impl RegistryCredentials {
    fn default() -> Self {
        Self::Anonymous
    }
}

impl RegistryCredentials {
    /// Create credentials from explicit username/password.
    pub fn basic(username: String, password: String) -> Self {
        Self::Basic { username, password }
    }

    /// Convert to `oci_distribution::secrets::RegistryAuth`.
    pub fn to_registry_auth(&self) -> oci_distribution::secrets::RegistryAuth {
        match self {
            RegistryCredentials::Anonymous => oci_distribution::secrets::RegistryAuth::Anonymous,
            RegistryCredentials::Basic { username, password } => {
                oci_distribution::secrets::RegistryAuth::Basic(username.clone(), password.clone())
            }
        }
    }

    /// Get username if basic auth.
    pub fn username(&self) -> Option<&str> {
        match self {
            RegistryCredentials::Anonymous => None,
            RegistryCredentials::Basic { username, .. } => Some(username),
        }
    }

    /// Get password if basic auth.
    pub fn password(&self) -> Option<&str> {
        match self {
            RegistryCredentials::Anonymous => None,
            RegistryCredentials::Basic { password, .. } => Some(password),
        }
    }
}

/// Resolve credentials for a registry from `~/.docker/config.json`.
/// Falls back to anonymous if no config or no matching entry is found.
pub fn resolve_credentials(registry: &str) -> RegistryCredentials {
    let Some(config_path) = find_docker_config() else {
        debug!(
            registry = registry,
            "No Docker config found, using anonymous access"
        );
        return RegistryCredentials::default();
    };

    debug!(path = %config_path.display(), "Found Docker config");

    let config = match DockerConfig::load(&config_path) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to parse Docker config");
            return RegistryCredentials::default();
        }
    };

    if let Some(creds) = config.get_auth(registry) {
        debug!(registry = registry, "Found credentials in Docker config");
        return creds;
    }

    debug!(
        registry = registry,
        "No credentials found, using anonymous access"
    );
    RegistryCredentials::default()
}

/// Find the Docker config.json file.
/// Checks DOCKER_CONFIG env var first, then ~/.docker/config.json.
fn find_docker_config() -> Option<PathBuf> {
    if let Ok(docker_config) = std::env::var("DOCKER_CONFIG") {
        let path = Path::new(&docker_config).join("config.json");
        if path.exists() {
            return Some(path);
        }
    }

    if let Some(home) = dirs::home_dir() {
        let path = home.join(".docker").join("config.json");
        if path.exists() {
            return Some(path);
        }
    }

    None
}

/// Parsed Docker config.json.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DockerConfig {
    #[serde(default)]
    pub auths: HashMap<String, AuthEntry>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct AuthEntry {
    #[serde(default)]
    pub auth: Option<String>,
}

impl DockerConfig {
    /// Load and parse a Docker config.json file.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let data = std::fs::read_to_string(path)?;
        let config: DockerConfig = serde_json::from_str(&data)?;
        Ok(config)
    }

    /// Save the Docker config.json file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(self)?;
        std::fs::write(path, data)?;
        Ok(())
    }

    /// Get credentials from the auths section for a registry.
    /// Tries exact match first, then tries with/without https:// prefix.
    pub fn get_auth(&self, registry: &str) -> Option<RegistryCredentials> {
        let candidates = [
            registry.to_string(),
            format!("https://{}", registry),
            format!("https://{}/v1/", registry),
            format!("https://{}/v2/", registry),
        ];

        for candidate in &candidates {
            if let Some(entry) = self.auths.get(candidate) {
                match entry_to_credentials(entry) {
                    Ok(creds) => return Some(creds),
                    Err(e) => {
                        warn!(registry = %candidate, error = %e, "Failed to parse credentials");
                    }
                }
            }
        }

        // Also try stripping https:// from stored keys
        for (key, entry) in &self.auths {
            let stripped = key
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            if stripped == registry {
                match entry_to_credentials(entry) {
                    Ok(creds) => return Some(creds),
                    Err(e) => {
                        warn!(registry = %key, error = %e, "Failed to parse credentials");
                    }
                }
            }
        }

        None
    }

    /// Set credentials in the auths section for a registry.
    pub fn set_auth(&mut self, registry: &str, username: &str, password: &str) {
        let token =
            base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", username, password));
        self.auths
            .insert(registry.to_string(), AuthEntry { auth: Some(token) });
    }

    /// Remove credentials for a registry from the auths section.
    pub fn remove_auth(&mut self, registry: &str) -> bool {
        self.auths.remove(registry).is_some()
    }
}

/// Decode a base64 "auth" field (format: "username:password") into RegistryCredentials.
fn entry_to_credentials(entry: &AuthEntry) -> Result<RegistryCredentials, anyhow::Error> {
    let auth = entry
        .auth
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("No auth field found"))?;

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(auth)
        .map_err(|e| anyhow::anyhow!("Invalid base64 encoding: {}", e))?;

    let decoded_str = String::from_utf8(decoded)
        .map_err(|e| anyhow::anyhow!("Invalid UTF-8 in decoded auth: {}", e))?;

    let (user, pass) = decoded_str
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("Auth field missing ':' separator"))?;

    if user.is_empty() || pass.is_empty() {
        return Err(anyhow::anyhow!("Username or password cannot be empty"));
    }

    Ok(RegistryCredentials::basic(
        user.to_string(),
        pass.to_string(),
    ))
}

/// Extract the registry hostname from an image reference.
/// e.g., "docker.io/library/alpine:latest" -> "docker.io"
/// e.g., "ghcr.io/user/repo:v1" -> "ghcr.io"
/// e.g., "alpine:latest" -> "docker.io"
pub fn registry_from_image_ref(image_ref: &str) -> String {
    // If it contains a dot before the first slash, it has an explicit registry
    if let Some(slash_pos) = image_ref.find('/') {
        let potential_registry = &image_ref[..slash_pos];
        if potential_registry.contains('.') || potential_registry.contains(':') {
            return potential_registry.to_string();
        }
    }
    // Default to Docker Hub
    "docker.io".to_string()
}

/// Get the default Docker config.json path.
pub fn docker_config_path() -> Option<PathBuf> {
    find_docker_config().or_else(|| dirs::home_dir().map(|h| h.join(".docker").join("config.json")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_registry_auth_anonymous() {
        let creds = RegistryCredentials::default();
        let auth = creds.to_registry_auth();
        assert!(matches!(
            auth,
            oci_distribution::secrets::RegistryAuth::Anonymous
        ));
    }

    #[test]
    fn test_to_registry_auth_basic() {
        let creds = RegistryCredentials::basic("user".into(), "pass".into());
        let auth = creds.to_registry_auth();
        assert!(matches!(
            auth,
            oci_distribution::secrets::RegistryAuth::Basic(_, _)
        ));
    }

    #[test]
    fn test_registry_from_image_ref() {
        assert_eq!(registry_from_image_ref("alpine:latest"), "docker.io");
        assert_eq!(
            registry_from_image_ref("library/alpine:latest"),
            "docker.io"
        );
        assert_eq!(
            registry_from_image_ref("docker.io/library/alpine:latest"),
            "docker.io"
        );
        assert_eq!(registry_from_image_ref("ghcr.io/user/repo:v1"), "ghcr.io");
        assert_eq!(
            registry_from_image_ref("myregistry.com:5000/repo"),
            "myregistry.com:5000"
        );
    }

    #[test]
    fn test_docker_config_get_auth() {
        let mut config = DockerConfig::default();
        config.set_auth("docker.io", "testuser", "testpass");

        let creds = config.get_auth("docker.io").unwrap();
        assert_eq!(creds.username().unwrap(), "testuser");
        assert_eq!(creds.password().unwrap(), "testpass");
    }

    #[test]
    fn test_docker_config_get_auth_with_https_prefix() {
        let mut config = DockerConfig::default();
        config.auths.insert(
            "https://index.docker.io/v1/".to_string(),
            AuthEntry {
                auth: Some(base64::engine::general_purpose::STANDARD.encode("myuser:mypass")),
            },
        );

        let creds = config.get_auth("index.docker.io").unwrap();
        assert_eq!(creds.username().unwrap(), "myuser");
        assert_eq!(creds.password().unwrap(), "mypass");
    }

    #[test]
    fn test_docker_config_remove_auth() {
        let mut config = DockerConfig::default();
        config.set_auth("docker.io", "testuser", "testpass");
        assert!(config.get_auth("docker.io").is_some());

        assert!(config.remove_auth("docker.io"));
        assert!(config.get_auth("docker.io").is_none());
        assert!(!config.remove_auth("docker.io"));
    }

    #[test]
    fn test_docker_config_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        let mut config = DockerConfig::default();
        config.set_auth("docker.io", "user1", "pass1");
        config.set_auth("ghcr.io", "user2", "pass2");

        config.save(&path).unwrap();
        let loaded = DockerConfig::load(&path).unwrap();

        assert!(loaded.get_auth("docker.io").is_some());
        assert!(loaded.get_auth("ghcr.io").is_some());
    }

    #[test]
    fn test_resolve_credentials_falls_back_to_anonymous() {
        // With no Docker config available, should fall back to anonymous
        let resolved = resolve_credentials("nonexistent.registry.io");
        assert!(resolved.username().is_none());
        assert!(resolved.password().is_none());
    }
}
