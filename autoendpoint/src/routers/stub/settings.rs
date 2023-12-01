/// Settings for `StubRouter`
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct StubSettings {
    pub url: String,
    #[serde(rename = "credentials")]
    pub server_credentials: String,
}

/// Credential information for each application
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct StubServerSettings {
    pub project_id: String,
    pub error: String,
}

impl Default for StubSettings {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080".to_owned(),
            server_credentials: "{\"project_id\":\"test\", \"error\":\"\"}".to_string(),
        }
    }
}
