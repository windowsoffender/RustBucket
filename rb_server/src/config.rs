use serde::Deserialize;
use std::error::Error;
use std::path::{Path, PathBuf};

/// Top-level server configuration. Loaded from a TOML file, then overridden by CLI flags.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct RbServerConfig {
    pub host: String,
    pub port: u16,
    pub verbose: bool,
    /// Path to the SQLite database. Empty string keeps state in memory only.
    pub db_path: String,
    pub mtls: MtlsConfig,
}

/// Mutual TLS settings for the operator channel.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MtlsConfig {
    pub enabled: bool,
    pub ca_path: String,
    pub ca_key_path: String,
    pub cert_path: String,
    pub key_path: String,
    pub crl_path: String,
    pub crl_update_seconds: u64,
}

impl Default for RbServerConfig {
    fn default() -> Self {
        RbServerConfig {
            host: "0.0.0.0".to_string(),
            port: 6666,
            verbose: false,
            db_path: "rustbucket.sqlite".to_string(),
            mtls: MtlsConfig::default(),
        }
    }
}

impl Default for MtlsConfig {
    fn default() -> Self {
        MtlsConfig {
            enabled: false,
            ca_path: "certs/ca-cert.pem".to_string(),
            ca_key_path: "certs/ca-key.pem".to_string(),
            cert_path: "certs/client-cert.pem".to_string(),
            key_path: "certs/client-key.pem".to_string(),
            crl_path: "certs/crl.der".to_string(),
            crl_update_seconds: 5,
        }
    }
}

impl RbServerConfig {
    /// Load the configuration.
    ///
    /// An explicit `path` must exist. When `path` is `None`, `./rb_server.toml` is used if present,
    /// otherwise the built-in defaults.
    pub fn load(path: Option<&Path>) -> Result<Self, Box<dyn Error>> {
        match path {
            Some(path) => Self::from_file(path),
            None => {
                let default_path = PathBuf::from("rb_server.toml");
                if default_path.exists() {
                    Self::from_file(&default_path)
                } else {
                    Ok(Self::default())
                }
            }
        }
    }

    fn from_file(path: &Path) -> Result<Self, Box<dyn Error>> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read config file {}: {}", path.display(), e))?;
        toml::from_str(&contents)
            .map_err(|e| format!("failed to parse config file {}: {}", path.display(), e).into())
    }
}
