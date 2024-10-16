use std::io::Read;

use log::error;
use serde::Deserialize;

use crate::{api_access::ApiAccessConfig, connection::ServerConfig};

const DEFAULT_CONFIG_PATH: &str = "config.toml";

#[derive(Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(flatten)]
    pub api_access: ApiAccessConfig,

    #[serde(flatten)]
    pub server: ServerConfig,
}

pub fn read_config(file: &mut impl Read) -> Config {
    let mut contents = String::new();
    if let Err(err) = file.read_to_string(&mut contents) {
        error!("Failed to read config: {err}. Reverting to default config.",);
        return Config::default();
    };

    match toml::from_str(&contents) {
        Ok(config) => config,
        Err(err) => {
            error!("Failed to parse config file: {err}. Reverting to default config.");
            Config::default()
        }
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
        let config = read_config(&mut config_file);

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
}
