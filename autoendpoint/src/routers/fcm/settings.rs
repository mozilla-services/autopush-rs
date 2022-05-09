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
    /// {"_project_id_":{"project_id": "_project_id_", "credential": "_key_"}, ...}
    /// ```
    /// For FCM, `credential` keys can be either a serialized JSON string, or the
    /// path to the JSON key file.
    ///
    /// GCM follows the same pattern, where
    ///
    /// `_project_id_` == senderID and `_key_` == credential
    /// e.g. "bar-project" is via FCM, with a serialized JSON key,
    /// e.g. "gorp-project" is via FCM, with a key path,
    /// and a GCM project with SenderID of "f00" specifies the server key as credential
    ///
    /// ```json
    /// {"bar-project":{"project_id": "bar-project-1234", "credential": "{\"type\": ...}"},
    ///  "gorp-project":{"project_id": "gorp-project-abcd", "credential": "keys/gorp-project.json"},
    ///  "f00": {"project_id": "f00", "credential": "abcd0123457"},
    ///  ...
    /// }
    /// ```
    #[serde(rename = "credentials")]
    pub server_credentials: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
    /// The base URL to use for FCM requests
    pub base_url: Url,
    /// The number of seconds to wait for FCM requests to complete
    pub timeout: usize,
}

/// Credential information for each application
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct FcmServerCredential {
    pub project_id: String,
    #[serde(rename = "credential")]
    pub server_access_token: String,
}

impl Default for FcmSettings {
    fn default() -> Self {
        Self {
            min_ttl: 60,
            server_credentials: "{}".to_string(),
            max_data: 4096,
            base_url: Url::parse("https://fcm.googleapis.com").unwrap(),
            timeout: 3,
        }
    }
}

impl FcmSettings {
    /// Read the credentials from the provided JSON
    pub fn credentials(&self) -> serde_json::Result<HashMap<String, FcmServerCredential>> {
        serde_json::from_str(&self.server_credentials)
    }
}
