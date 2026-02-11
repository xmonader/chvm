use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Relevant parts of the OCI image configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageConfig {
    #[serde(default)]
    pub cmd: Vec<String>,
    #[serde(default)]
    pub entrypoint: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub working_dir: String,
    #[serde(default)]
    pub user: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
}

impl ImageConfig {
    /// Parse from a raw OCI config JSON value.
    pub fn from_oci_config(value: &serde_json::Value) -> Option<Self> {
        let config = value.get("config")?;

        Some(ImageConfig {
            cmd: parse_string_array(config.get("Cmd")),
            entrypoint: parse_string_array(config.get("Entrypoint")),
            env: parse_string_array(config.get("Env")),
            working_dir: config
                .get("WorkingDir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            user: config
                .get("User")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            labels: config
                .get("Labels")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default(),
        })
    }
}

fn parse_string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_string_array_valid() {
        let value = json!(["cmd1", "cmd2", "cmd3"]);
        let result = parse_string_array(Some(&value));
        assert_eq!(result, vec!["cmd1", "cmd2", "cmd3"]);
    }

    #[test]
    fn test_parse_string_array_empty() {
        let value = json!([]);
        let result = parse_string_array(Some(&value));
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn test_parse_string_array_none() {
        let result = parse_string_array(None);
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn test_parse_string_array_mixed_types() {
        let value = json!(["string", 123, true, "another"]);
        let result = parse_string_array(Some(&value));
        assert_eq!(result, vec!["string", "another"]);
    }

    #[test]
    fn test_image_config_from_oci_config() {
        let oci_config = json!({
            "config": {
                "Cmd": ["/bin/sh"],
                "Entrypoint": ["/entrypoint.sh"],
                "Env": ["PATH=/usr/bin", "HOME=/root"],
                "WorkingDir": "/app",
                "User": "1000:1000",
                "Labels": {
                    "version": "1.0",
                    "maintainer": "test@example.com"
                }
            }
        });

        let config = ImageConfig::from_oci_config(&oci_config).unwrap();
        assert_eq!(config.cmd, vec!["/bin/sh"]);
        assert_eq!(config.entrypoint, vec!["/entrypoint.sh"]);
        assert_eq!(config.env, vec!["PATH=/usr/bin", "HOME=/root"]);
        assert_eq!(config.working_dir, "/app");
        assert_eq!(config.user, "1000:1000");
        assert_eq!(config.labels.get("version"), Some(&"1.0".to_string()));
        assert_eq!(
            config.labels.get("maintainer"),
            Some(&"test@example.com".to_string())
        );
    }

    #[test]
    fn test_image_config_from_oci_config_minimal() {
        let oci_config = json!({
            "config": {}
        });

        let config = ImageConfig::from_oci_config(&oci_config).unwrap();
        assert_eq!(config.cmd, Vec::<String>::new());
        assert_eq!(config.entrypoint, Vec::<String>::new());
        assert_eq!(config.env, Vec::<String>::new());
        assert_eq!(config.working_dir, "");
        assert_eq!(config.user, "");
        assert!(config.labels.is_empty());
    }

    #[test]
    fn test_image_config_from_oci_config_no_config_field() {
        let oci_config = json!({
            "architecture": "amd64"
        });

        let config = ImageConfig::from_oci_config(&oci_config);
        assert!(config.is_none());
    }

    #[test]
    fn test_image_config_serialization() {
        let mut labels = HashMap::new();
        labels.insert("key".to_string(), "value".to_string());

        let config = ImageConfig {
            cmd: vec!["/bin/sh".to_string()],
            entrypoint: vec![],
            env: vec!["VAR=value".to_string()],
            working_dir: "/app".to_string(),
            user: "root".to_string(),
            labels,
        };

        let json = serde_json::to_string(&config).unwrap();
        let deserialized: ImageConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.cmd, config.cmd);
        assert_eq!(deserialized.env, config.env);
        assert_eq!(deserialized.working_dir, config.working_dir);
        assert_eq!(deserialized.user, config.user);
        assert_eq!(deserialized.labels, config.labels);
    }

    #[test]
    fn test_image_config_defaults() {
        let config = ImageConfig {
            cmd: vec![],
            entrypoint: vec![],
            env: vec![],
            working_dir: String::new(),
            user: String::new(),
            labels: HashMap::new(),
        };

        assert!(config.cmd.is_empty());
        assert!(config.entrypoint.is_empty());
        assert!(config.env.is_empty());
        assert!(config.working_dir.is_empty());
        assert!(config.user.is_empty());
        assert!(config.labels.is_empty());
    }
}
