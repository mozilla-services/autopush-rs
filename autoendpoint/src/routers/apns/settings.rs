use std::collections::HashMap;
use std::path::PathBuf;

/// Settings for `ApnsRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ApnsSettings {
    pub channels: String,
}

/// Settings for a specific APNS release channel
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ApnsChannelSettings {
    pub cert: PathBuf,
    pub password: String,
    pub topic: Option<String>,
    pub sandbox: bool,
}

impl Default for ApnsSettings {
    fn default() -> Self {
        Self {
            channels: "{}".to_string(),
        }
    }
}

impl ApnsSettings {
    /// Read the channels from the JSON string
    pub fn channels(&self) -> serde_json::Result<HashMap<String, ApnsChannelSettings>> {
        serde_json::from_str(&self.channels)
    }
}
