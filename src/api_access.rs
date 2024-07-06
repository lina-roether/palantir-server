use std::sync::Arc;

use serde::Deserialize;

use crate::config::Config;

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ApiPermissions {
    pub connect: bool,
    pub host: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct ApiKey {
    key: String,

    #[serde(default = "ApiPermissions::connect", flatten)]
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
            connect: false,
            host: false,
        }
    }

    pub const fn connect() -> Self {
        Self {
            connect: true,
            host: false,
        }
    }

    pub const fn host() -> Self {
        Self {
            connect: false,
            host: true,
        }
    }

    pub const fn all() -> Self {
        Self {
            connect: true,
            host: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
struct ApiAccessPolicy {
    restrict_connect: bool,
    restrict_host: bool,
}

impl Default for ApiAccessPolicy {
    fn default() -> Self {
        Self {
            restrict_connect: true,
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

        let default_perms = ApiPermissions {
            connect: !config.policy.restrict_connect,
            host: !config.policy.restrict_host,
        };

        let Some(key) = key else {
            return default_perms;
        };

        let Some(key_config) = config.keys.iter().find(|k| k.key == key) else {
            return default_perms;
        };

        ApiPermissions {
            connect: !config.policy.restrict_connect || key_config.permissions.connect,
            host: !config.policy.restrict_host || key_config.permissions.host,
        }
    }
}
