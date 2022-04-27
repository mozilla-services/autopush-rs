use std::collections::HashMap;

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
    /// This contains both GCM and FCM credentials.
    /// FCM (the more modern credential set) is specified as
    ///
    /// ```json
    /// {"_project_id_":{"projectid": "_project_id_", "auth": "_key_"}, ...}
    /// ```
    ///
    /// GCM follows the same pattern, where
    ///
    /// `_project_id_` == senderID and `_key_` == auth
    /// e.g. for a FCM of "bar-project" and a GCM of "f00"
    ///
    /// ```json
    /// {"bar-project":{"projectid": "bar-project-1234", "auth": "keys/bar-project-12345abcd.json"},
    ///  "f00": {"projectid": "f00", "auth": "abcd0123457"},
    ///  ...
    /// }
    /// ```
    pub credentials: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
    /// The base URL to use for FCM requests
    pub base_url: Url,
    /// The number of seconds to wait for FCM requests to complete
    pub timeout: usize,
}

/// Credential information for each application
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct FcmCredential {
    pub project_id: String,
    pub credential: String,
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
