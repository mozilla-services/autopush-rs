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
    /// {"_project_id_":{"project_id": "_project_id_", "auth": "_key_"}, ...}
    /// ```
    /// For FCM, `auth` keys can be either a serialized JSON string, or the
    /// path to the JSON key file.
    ///
    /// GCM follows the same pattern, where
    ///
    /// `_project_id_` == senderID and `_key_` == auth
    /// e.g. "bar-project" is via FCM, with a serialized JSON key,
    /// e.g. "gorp-project" is via FCM, a and a GCM of "f00"
    ///
    /// ```json
    /// {"bar-project":{"project_id": "bar-project-1234", "auth": "{\"type\": ...}"},
    ///  "gorp-project":{"project_id": "gorp-project-abcd", "auth": "keys/gorp-project.json"},
    ///  "f00": {"project_id": "f00", "auth": "abcd0123457"},
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
