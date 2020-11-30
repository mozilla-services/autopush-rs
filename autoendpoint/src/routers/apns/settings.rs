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
}

/// Settings for a specific APNS release channel
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ApnsChannel {
    /// the cert and key are either fully qualified paths (starts with "/")
    /// or an inline value.
    pub cert: String,
    pub key: String,
    pub topic: Option<String>,
    pub sandbox: bool,
}

impl Default for ApnsSettings {
    fn default() -> Self {
        Self {
            channels: "{}".to_string(),
            max_data: 4096,
        }
    }
}

impl ApnsSettings {
    /// Read the channels from the JSON string
    pub fn channels(&self) -> serde_json::Result<HashMap<String, ApnsChannel>> {
        serde_json::from_str(&self.channels)
    }
}
