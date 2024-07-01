use std::{collections::HashMap, fs, path::PathBuf, str::FromStr};

use log::error;
use serde::Deserialize;

const DEFAULT_CONFIG_PATH: &str = "./access.toml";

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ApiPermissions {
    pub join: bool,
    pub host: bool,
}

#[derive(Debug, Deserialize)]
struct ApiKey {
    key: String,

    #[serde(default, flatten)]
    permissions: ApiPermissions,
}

#[allow(clippy::derivable_impls)]
impl Default for ApiPermissions {
    fn default() -> Self {
        Self {
            join: false,
            host: false,
        }
    }
}

impl ApiPermissions {
    pub const fn join() -> Self {
        Self {
            join: true,
            host: false,
        }
    }

    pub const fn host() -> Self {
        Self {
            join: false,
            host: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
struct ApiAccessPolicy {
    disable_access_control: bool,
    restrict_join: bool,
    restrict_host: bool,
}

impl Default for ApiAccessPolicy {
    fn default() -> Self {
        Self {
            disable_access_control: false,
            restrict_join: true,
            restrict_host: true,
        }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct ApiAccessConfig {
    policy: ApiAccessPolicy,
    keys: Vec<ApiKey>,
}

pub struct ApiAccessManager {
    config_path: PathBuf,
}

impl ApiAccessManager {
    pub fn new(config_path: Option<PathBuf>) -> Self {
        Self {
            config_path: config_path.unwrap_or_else(|| {
                PathBuf::from_str(DEFAULT_CONFIG_PATH)
                    .expect("Failed to construct default config path!")
            }),
        }
    }

    fn get_config(&self) -> ApiAccessConfig {
        let cfg_file = match fs::read_to_string(&self.config_path) {
            Ok(cfg_file) => cfg_file,
            Err(err) => {
                error!("Failed to open {}: {}", self.config_path.display(), err);
                return ApiAccessConfig::default();
            }
        };

        let config: ApiAccessConfig = match toml::from_str(&cfg_file) {
            Ok(config) => config,
            Err(err) => {
                error!("Failed to parse {}: {}", self.config_path.display(), err);
                return ApiAccessConfig::default();
            }
        };

        config
    }

    pub fn acquire_permissions(&self, key: Option<&str>, permissions: ApiPermissions) -> bool {
        let config = self.get_config();

        if config.policy.disable_access_control {
            return true;
        }

        let Some(key) = key else {
            return false;
        };

        let mut join_check = !permissions.join;
        let mut host_check = !permissions.host;

        join_check |= !config.policy.restrict_join;
        host_check |= !config.policy.restrict_host;

        if let Some(key_config) = config.keys.iter().find(|k| k.key == key) {
            join_check |= key_config.permissions.join;
            host_check |= key_config.permissions.host;
        }

        join_check && host_check
    }
}
