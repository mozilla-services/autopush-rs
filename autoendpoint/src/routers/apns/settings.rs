use std::collections::HashMap;

/// Settings for `ApnsRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
//#[serde(deny_unknown_fields)] // Allow unknown fields so we can add comments to the secrets.
pub struct ApnsSettings {
    /// A JSON dict of `ApnsChannel`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, ApnsChannel>`
    pub channels: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
    // These values correspond to the a2 library ClientConfig struct.
    // https://github.com/WalletConnect/a2/blob/master/src/client.rs#L65-L71.
    // Utilized by apns router config in creating the client.
    pub request_timeout_secs: Option<u64>,
    pub pool_idle_timeout_secs: Option<u64>,
}

/// Settings for a specific APNS release channel
///
/// Two authentication modes are supported:
/// - Token-based auth (preferred): set `key` to the APNS provider auth key
///   (`.p8`), and set both `key_id` and `team_id`. `cert` is unused.
/// - Certificate-based auth: set `cert` and `key` to the provider
///   certificate/key pair, and leave `key_id`/`team_id` unset.
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
//#[serde(deny_unknown_fields)] // Allow unknown fields so we can add comments to the secrets.
pub struct ApnsChannel {
    /// the cert and key are either paths
    /// or an inline value that starts with "-"
    /// e.g. `-----BEGIN PRIVATE KEY-----\n`
    pub cert: String,
    pub key: String,
    /// The 10-character Apple key ID for the `.p8` auth key (token-based auth)
    pub key_id: Option<String>,
    /// The 10-character Apple Developer team ID (token-based auth)
    pub team_id: Option<String>,
    pub topic: Option<String>,
    pub sandbox: bool,
}

impl Default for ApnsSettings {
    fn default() -> ApnsSettings {
        ApnsSettings {
            channels: "{}".to_string(),
            max_data: 4096,
            request_timeout_secs: Some(20),
            pool_idle_timeout_secs: Some(600),
        }
    }
}

impl ApnsSettings {
    /// Read the channels from the JSON string
    pub fn channels(&self) -> serde_json::Result<HashMap<String, ApnsChannel>> {
        serde_json::from_str(&self.channels)
    }
}
