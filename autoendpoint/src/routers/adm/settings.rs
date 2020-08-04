use std::collections::HashMap;
use url::Url;

/// Settings for `AdmRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AdmSettings {
    /// A JSON dict of `AdmProfile`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, AdmProfile>`
    pub profiles: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
    /// The base URL to use for ADM requests
    pub base_url: Url,
    /// The number of seconds to wait for ADM requests to complete
    pub timeout: usize,
    /// The minimum TTL to use for ADM notifications
    pub min_ttl: usize,
}

/// Settings for a specific ADM profile
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AdmProfile {
    pub client_id: String,
    pub client_secret: String,
}

impl Default for AdmSettings {
    fn default() -> Self {
        Self {
            profiles: "{}".to_string(),
            max_data: 6000,
            base_url: Url::parse("https://api.amazon.com").unwrap(),
            timeout: 3,
            min_ttl: 60,
        }
    }
}

impl AdmSettings {
    /// Read the profiles from the JSON string
    pub fn profiles(&self) -> serde_json::Result<HashMap<String, AdmProfile>> {
        serde_json::from_str(&self.profiles)
    }
}
