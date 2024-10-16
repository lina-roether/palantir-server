use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ApiPermissions {
    pub connect: bool,
    pub host: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ApiKey {
    pub key: String,

    #[serde(default = "ApiPermissions::connect", flatten)]
    pub permissions: ApiPermissions,
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

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct ApiAccessPolicy {
    pub restrict_connect: bool,
    pub restrict_host: bool,
}

impl Default for ApiAccessPolicy {
    fn default() -> Self {
        Self {
            restrict_connect: true,
            restrict_host: true,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Default, Clone)]
#[serde(default)]
pub struct ApiAccessConfig {
    pub api_policy: ApiAccessPolicy,
    pub api_keys: Vec<ApiKey>,
}

pub struct ApiAccessManager {
    config: ApiAccessConfig,
}

impl ApiAccessManager {
    pub fn new(config: ApiAccessConfig) -> Self {
        Self { config }
    }

    pub fn get_permissions(&self, key: Option<&str>) -> ApiPermissions {
        let default_perms = ApiPermissions {
            connect: !self.config.api_policy.restrict_connect,
            host: !self.config.api_policy.restrict_host,
        };

        let Some(key) = key else {
            return default_perms;
        };

        let Some(key_config) = self.config.api_keys.iter().find(|k| k.key == key) else {
            return default_perms;
        };

        ApiPermissions {
            connect: !self.config.api_policy.restrict_connect || key_config.permissions.connect,
            host: !self.config.api_policy.restrict_host || key_config.permissions.host,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_fallback_to_policy_without_key() {
        // given
        let config = ApiAccessConfig {
            api_policy: ApiAccessPolicy {
                restrict_connect: false,
                restrict_host: true,
            },
            ..ApiAccessConfig::default()
        };
        let manager = ApiAccessManager::new(config);

        // when
        let permissions = manager.get_permissions(None);

        // then
        assert_eq!(permissions, ApiPermissions::connect())
    }

    #[test]
    fn should_fallback_to_policy_with_invalid_key() {
        // given
        let config = ApiAccessConfig {
            api_policy: ApiAccessPolicy {
                restrict_host: true,
                restrict_connect: true,
            },
            api_keys: vec![ApiKey {
                key: "AAAAA".to_string(),
                permissions: ApiPermissions::all(),
            }],
        };
        let manager = ApiAccessManager::new(config);

        // when
        let permissions = manager.get_permissions(Some("BBBBB"));

        // then
        assert_eq!(permissions, ApiPermissions::none())
    }

    #[test]
    fn should_use_key_permissions_with_correct_key() {
        // given
        let config = ApiAccessConfig {
            api_policy: ApiAccessPolicy {
                restrict_host: true,
                restrict_connect: true,
            },
            api_keys: vec![ApiKey {
                key: "AAAAA".to_string(),
                permissions: ApiPermissions::all(),
            }],
        };
        let manager = ApiAccessManager::new(config);

        // when
        let permissions = manager.get_permissions(Some("AAAAA"));

        // then
        assert_eq!(permissions, ApiPermissions::all());
    }
}
