use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_host")]
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_buckets_dir")]
    pub buckets_dir: String,
}

fn default_host() -> String {
    "0.0.0.0".to_string()
}

fn default_port() -> u16 {
    8080
}

fn default_buckets_dir() -> String {
    "./buckets".to_string()
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: default_host(),
            port: default_port(),
            buckets_dir: default_buckets_dir(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerConfig,
}

impl AppConfig {
    pub fn load(path: Option<&Path>) -> Result<Self, Box<dyn std::error::Error>> {
        let path = path.unwrap_or(Path::new("stathost.toml"));
        if path.exists() {
            let content = std::fs::read_to_string(path)?;
            Ok(toml::from_str(&content)?)
        } else {
            Ok(AppConfig {
                server: ServerConfig::default(),
            })
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct BucketAuth {
    pub token: String,
}

#[derive(Debug, Deserialize)]
pub struct BucketConfig {
    pub auth: BucketAuth,
}

impl BucketConfig {
    pub fn load(bucket_path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let config_path = bucket_path.join("config.toml");
        let content = std::fs::read_to_string(config_path)?;
        Ok(toml::from_str(&content)?)
    }
}
