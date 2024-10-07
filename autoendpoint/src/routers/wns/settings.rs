use std::collections::HashMap;

use url::Url;

/// Settings for `WnsRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct WnsSettings {
    /// The minimum TTL to use for WNS notifications
    pub min_ttl: usize,
    /// A JSON dict of `WnsCredential`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, WnsCredential>`
    /// WNS is specified as
    ///
    /// ```json
    /// {"_project_id_":{"project_id": "_project_id_", "credential": "_key_"}, ...}
    /// ```
    /// For WNS, `credential` keys can be either a serialized JSON string, or the
    /// path to the JSON key file.
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
    /// The base URL to use for WNS requests
    pub base_url: Url,
    /// The number of seconds to wait for WNS requests to complete
    pub timeout: usize,
}

/// Credential information for each application
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct WnsServerCredential {
    pub project_id: String,
    #[serde(rename = "credential")]
    pub server_access_token: String,
}

impl Default for WnsSettings {
    fn default() -> Self {
        Self {
            min_ttl: 60,
            server_credentials: "{}".to_string(),
            max_data: 4096,
            base_url: Url::parse("https://login.microsoftonline.com").unwrap(),
            timeout: 3,
        }
    }
}

impl WnsSettings {
    /// Read the credentials from the provided JSON
    pub fn credentials(&self) -> serde_json::Result<HashMap<String, WnsServerCredential>> {
        trace!("credentials: {}", self.server_credentials);
        serde_json::from_str(&self.server_credentials)
    }
}
