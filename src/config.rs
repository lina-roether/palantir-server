use std::{
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::Context;
use serde::Deserialize;

use crate::{api_access::ApiAccessConfig, app::Cli, connection::ServerConfig};

const DEFAULT_CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(flatten)]
    pub api_access: ApiAccessConfig,

    #[serde(flatten)]
    pub server: ServerConfig,
}

impl Config {
    pub fn read(file: &mut impl Read) -> anyhow::Result<Self> {
        let mut contents = String::new();
        file.read_to_string(&mut contents)
            .context("Failed to read config file")?;

        let config = toml::from_str(&contents).context("Failed to parse config file")?;
        Ok(config)
    }

    pub fn read_path(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let mut file = File::open(path).context("Failed to open config file")?;
        Self::read(&mut file)
    }

    pub fn from_cli_args(args: &Cli) -> anyhow::Result<Self> {
        let mut config = match &args.config {
            Some(config_path) => Self::read_path(config_path)?,
            None => {
                let default_config = PathBuf::from(DEFAULT_CONFIG_PATH);
                if default_config.exists() {
                    log::info!("Using default config file {DEFAULT_CONFIG_PATH}");
                    Self::read_path(default_config)?
                } else {
                    log::warn!("No config file found; using default config");

                    #[cfg(debug_assertions)]
                    {
                        log::warn!("DEBUG DEFAULT CONFIG IS INSECURE! You are running a debug build, which uses an insecure default configuration for development purposes.");
                    }

                    Config::default()
                }
            }
        };
        if let Some(listen_on) = &args.listen_on {
            config.server.listen_on = listen_on.clone();
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::api_access::{ApiAccessPolicy, ApiKey, ApiPermissions};

    use super::*;

    const TEST_CONFIG: &str = r#"
listen_on = "127.0.0.1:6969"

[api_policy]
restrict_connect = false
restrict_host = true

[[api_keys]]
key = "AAAAA"
connect = true
host = true
"#;

    #[test]
    fn should_parse_config() {
        // given
        let mut config_file = Cursor::new(TEST_CONFIG);

        // when
        let config = Config::read(&mut config_file).unwrap();

        // then
        assert_eq!(
            config,
            Config {
                server: ServerConfig {
                    listen_on: "127.0.0.1:6969".to_string()
                },
                api_access: ApiAccessConfig {
                    api_policy: ApiAccessPolicy {
                        restrict_host: true,
                        restrict_connect: false
                    },
                    api_keys: vec![ApiKey {
                        key: "AAAAA".to_string(),
                        permissions: ApiPermissions::all()
                    }]
                },
            }
        )
    }

    #[test]
    fn should_return_error_on_invalid_syntax() {
        // given
        let mut config_file = Cursor::new("listen_on = ");

        // when
        let result = Config::read(&mut config_file);

        // then
        assert!(result.is_err());
    }
}
