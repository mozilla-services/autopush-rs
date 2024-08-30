use std::collections::HashMap;

/// Settings for `ApnsRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
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
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ApnsChannel {
    /// the cert and key are either paths
    /// or an inline value that starts with "-"
    /// e.g. `-----BEGIN PRIVATE KEY-----\n`
    pub cert: String,
    pub key: String,
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
