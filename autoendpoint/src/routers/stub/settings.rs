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

/// `StubServerSettings` allows the server configuration file to specify
/// the default error to use for requests to the "error" `app_id`.
/// Requests to endpoints associated with this client will return the
/// `error` string.
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

/// The `StubSettings` contains client provided data that can override
/// the response error string when endpoints associated with this client
/// are called.
impl Default for StubSettings {
    fn default() -> Self {
        Self {
            url: "http://localhost:8080".to_owned(),
            server_credentials: "{\"error\":\"General Error\"}".to_owned(),
        }
    }
}
