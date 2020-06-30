use std::collections::HashMap;
use std::path::PathBuf;

/// Settings for `FcmRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
pub struct FcmSettings {
    pub ttl: usize,
    pub validate_only: bool,
    pub collapse_key: String,
    /// A JSON dict of `FcmCredential`s. This must be a `String` because
    /// environment variables cannot encode a `HashMap<String, FcmCredential>`
    pub credentials: String,
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
            validate_only: false,
            collapse_key: "webpush".to_string(),
            credentials: "{}".to_string(),
        }
    }
}

impl FcmSettings {
    /// Read the credentials from the provided JSON
    pub fn credentials(&self) -> serde_json::Result<HashMap<String, FcmCredential>> {
        serde_json::from_str(&self.credentials)
    }
}
