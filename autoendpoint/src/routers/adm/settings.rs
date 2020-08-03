use std::collections::HashMap;

/// Settings for `AdmRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AdmSettings {
    /// A JSON dict of `AdmProfile`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, AdmProfile>`
    pub profiles: String,
    /// The max size of notification data in bytes
    pub max_data: usize,
}

/// Settings for a specific ADM profile
#[derive(Clone, Debug, Default, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct AdmProfile {
    pub client_id: String,
    pub client_secret: String,
}

impl Default for AdmSettings {
    fn default() -> Self {
        Self {
            profiles: "{}".to_string(),
            max_data: 6000,
        }
    }
}

impl AdmSettings {
    /// Read the profiles from the JSON string
    pub fn profiles(&self) -> serde_json::Result<HashMap<String, AdmProfile>> {
        serde_json::from_str(&self.profiles)
    }
}
