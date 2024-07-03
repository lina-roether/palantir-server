use std::sync::Arc;

use serde::Deserialize;

use crate::config::Config;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApiPermissions {
    pub join: bool,
    pub host: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiKey {
    key: String,

    #[serde(default, flatten)]
    permissions: ApiPermissions,
}

impl Default for ApiPermissions {
    fn default() -> Self {
        Self::none()
    }
}

impl ApiPermissions {
    pub const fn none() -> Self {
        Self {
            join: false,
            host: false,
        }
    }

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

    pub const fn all() -> Self {
        Self {
            join: true,
            host: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
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

#[derive(Debug, Deserialize, Default, Clone)]
#[serde(default)]
pub struct ApiAccessConfig {
    policy: ApiAccessPolicy,
    keys: Vec<ApiKey>,
}

pub struct ApiAccessManager {
    config: Arc<Config>,
}

impl ApiAccessManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    pub fn get_permissions(&self, key: Option<&str>) -> ApiPermissions {
        let config = &self.config.api_access;

        if config.policy.disable_access_control {
            return ApiPermissions::all();
        }

        let Some(key) = key else {
            return ApiPermissions::none();
        };

        let Some(key_config) = config.keys.iter().find(|k| k.key == key) else {
            return ApiPermissions::none();
        };

        ApiPermissions {
            join: !config.policy.restrict_join || key_config.permissions.join,
            host: !config.policy.restrict_host || key_config.permissions.host,
        }
    }
}
