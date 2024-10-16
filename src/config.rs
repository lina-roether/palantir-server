use std::{fs, path::PathBuf, str::FromStr};

use log::error;
use serde::Deserialize;

use crate::{api_access::ApiAccessConfig, connection::ServerConfig};

const DEFAULT_CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub api_access: ApiAccessConfig,
    pub server: ServerConfig,
}

pub fn read_config(path: Option<PathBuf>) -> Config {
    let path: PathBuf = path.unwrap_or_else(|| PathBuf::from_str(DEFAULT_CONFIG_PATH).unwrap());
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(err) => {
            error!(
                "Failed to read config at {}: {err}. Reverting to default config.",
                path.display()
            );
            return Config::default();
        }
    };

    match toml::from_str(&contents) {
        Ok(config) => config,
        Err(err) => {
            error!("Failed to parse config file: {err}. Reverting to default config.");
            Config::default()
        }
    }
}
