use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

/// Settings for `FcmRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct FcmSettings {
    /// The minimum TTL to use for FCM notifications
    pub min_ttl: usize,
    /// A JSON dict of `FcmCredential`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, FcmCredential>`
    pub credentials: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
    /// The base URL to use for FCM requests
    pub base_url: Url,
    /// The number of seconds to wait for FCM requests to complete
    pub timeout: usize,
}

/// Credential information for each application
#[derive(Clone, Debug, serde::Deserialize)]
pub struct FcmCredential {
    pub project_id: String,
    pub auth_file: PathBuf,
}

impl Default for FcmSettings {
    fn default() -> Self {
        Self {
            min_ttl: 60,
            credentials: "{}".to_string(),
            max_data: 4096,
            base_url: Url::parse("https://fcm.googleapis.com").unwrap(),
            timeout: 3,
        }
    }
}

impl FcmSettings {
    /// Read the credentials from the provided JSON
    pub fn credentials(&self) -> serde_json::Result<HashMap<String, FcmCredential>> {
        serde_json::from_str(&self.credentials)
    }
}
