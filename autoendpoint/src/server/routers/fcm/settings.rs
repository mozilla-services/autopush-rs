use std::collections::HashMap;
use std::path::PathBuf;
use url::Url;

/// Settings for `FcmRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub struct FcmSettings {
    pub ttl: usize,
    /// A JSON dict of `FcmCredential`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, FcmCredential>`
    pub credentials: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
    /// The base URL to use for FCM requests
    pub fcm_url: Url,
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
            ttl: 60,
            credentials: "{}".to_string(),
            max_data: 4096,
            fcm_url: Url::parse("https://fcm.googleapis.com").unwrap(),
        }
    }
}

impl FcmSettings {
    /// Read the credentials from the provided JSON
    pub fn credentials(&self) -> serde_json::Result<HashMap<String, FcmCredential>> {
        serde_json::from_str(&self.credentials)
    }
}
