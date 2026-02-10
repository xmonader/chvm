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
            working_dir: config.get("WorkingDir")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            user: config.get("User")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            labels: config.get("Labels")
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
