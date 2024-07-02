use std::{fs, path::PathBuf, str::FromStr, sync::Arc};

use log::error;
use serde::Deserialize;

use crate::{api_access::ApiAccessConfig, listener::ServerConfig};

const DEFAULT_CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api_access: ApiAccessConfig,
    pub server: ServerConfig,
}

pub fn read_config(path: Option<PathBuf>) -> Arc<Config> {
    let path: PathBuf = path.unwrap_or_else(|| PathBuf::from_str(DEFAULT_CONFIG_PATH).unwrap());
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) => {
            error!(
                "Failed to read config at {}: {err}. Reverting to default config.",
                path.display()
            );
            return Arc::new(Config::default());
        }
    };

    match toml::from_str(&contents) {
        Ok(config) => Arc::new(config),
        Err(err) => {
            error!("Failed to parse config file: {err}. Reverting to default config.");
            Arc::new(Config::default())
        }
    }
}
