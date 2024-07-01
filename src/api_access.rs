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

    pub fn acquire_permissions(&self, key: Option<&str>, permissions: ApiPermissions) -> bool {
        let config = &self.config.api_access;

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
