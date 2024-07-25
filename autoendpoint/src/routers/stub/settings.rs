/// Settings for `StubRouter`
/// These can be specified externally by the calling "client" during
/// registration and contain values that will be echoed back.
/// The `server_credentials` block is a JSON structure that contains
/// the default response for any routing request. This defaults to
/// 'error'.
#[derive(Clone, Debug, serde::Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct StubSettings {
    pub url: String,
    #[serde(rename = "credentials")]
    pub server_credentials: String,
}

/// StubServerSettings contains the default available client types.
///
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct StubServerSettings {
    #[serde(default)]
    pub error: String,
}

impl Default for StubServerSettings {
    fn default() -> Self {
        Self {
            error: "General Error".to_owned(),
        }
    }
}

impl Default for StubSettings {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080".to_owned(),
            server_credentials: "{\"error\":\"General Error\"}".to_owned(),
        }
    }
}
