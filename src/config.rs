use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub node: NodeConfig,
    pub audio: AudioConfig,
    pub transcription: TranscriptionConfig,
    pub storage: StorageConfig,
    pub sync: SyncConfig,
    pub api: ApiConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NodeConfig {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AudioConfig {
    pub memo_service_uuid: String,
    pub memo_characteristic_uuid: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TranscriptionConfig {
    pub model: String,
    #[serde(default = "default_threads")]
    pub threads: u32,
}

fn default_threads() -> u32 {
    4
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StorageConfig {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SyncConfig {
    pub grpc_port: u16,
    pub sync_interval: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
    pub websocket_port: u16,
    pub listen_address: String,
    #[serde(default)]
    pub https_endpoint: Option<String>,
}

impl Config {
    pub fn load() -> Result<Self> {
        let config_dir = Self::config_dir()?;
        std::fs::create_dir_all(&config_dir).context("Failed to create config directory")?;

        let mut builder = config::Config::builder()
            // Start with default config from the embedded file
            .add_source(config::File::from_str(
                include_str!("../config/default.toml"),
                config::FileFormat::Toml,
            ));

        // Override with user config if it exists
        let user_config_path = config_dir.join("config.toml");
        if user_config_path.exists() {
            builder = builder.add_source(config::File::from(user_config_path));
        }

        // Override with environment variables (MEMO_NODE_*)
        builder = builder.add_source(
            config::Environment::with_prefix("MEMO_NODE")
                .separator("_")
                .try_parsing(true),
        );

        let config = builder.build().context("Failed to build configuration")?;
        config
            .try_deserialize()
            .context("Failed to deserialize configuration")
    }

    pub fn config_dir() -> Result<PathBuf> {
        Ok(directories::ProjectDirs::from("", "", "memo-node")
            .context("Failed to determine config directory")?
            .config_dir()
            .to_path_buf())
    }

    pub fn data_dir() -> Result<PathBuf> {
        let dir = directories::ProjectDirs::from("", "", "memo-node")
            .context("Failed to determine data directory")?
            .data_dir()
            .to_path_buf();
        std::fs::create_dir_all(&dir).context("Failed to create data directory")?;
        Ok(dir)
    }

    pub fn storage_path(&self) -> Result<PathBuf> {
        let path = if self.storage.path.starts_with('~') {
            let home = directories::UserDirs::new()
                .context("Failed to determine home directory")?
                .home_dir()
                .to_path_buf();
            home.join(self.storage.path.trim_start_matches("~/"))
        } else {
            PathBuf::from(&self.storage.path)
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("Failed to create storage directory")?;
        }

        Ok(path)
    }
}
