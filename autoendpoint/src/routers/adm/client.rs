use crate::routers::adm::error::AdmError;
use crate::routers::adm::settings::{AdmProfile, AdmSettings};
use crate::routers::RouterError;
use autopush_common::util::sec_since_epoch;
use futures::lock::Mutex;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

/// Holds profile-specific ADM data and authentication. This client handles
/// sending notifications to ADM.
pub struct AdmClient {
    base_url: Url,
    profile: AdmProfile,
    timeout: Duration,
    http: reqwest::Client,
    token_info: Mutex<TokenInfo>,
}

/// Holds information about the cached access token
#[derive(Default)]
struct TokenInfo {
    token: String,
    expiration_time: u64,
}

/// A successful OAuth token response
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

/// A successful send response
#[derive(Deserialize)]
struct AdmSendResponse {
    #[serde(rename = "registrationID")]
    registration_id: String,
}

/// An error returned by ADM
#[derive(Deserialize)]
struct AdmResponseError {
    reason: Option<String>,
}

impl AdmClient {
    /// Create an `AdmClient` using the provided profile
    pub fn new(settings: &AdmSettings, profile: AdmProfile, http: reqwest::Client) -> Self {
        AdmClient {
            base_url: settings.base_url.clone(),
            profile,
            timeout: Duration::from_secs(settings.timeout as u64),
            http,
            // The default TokenInfo has dummy values to trigger a token fetch
            token_info: Mutex::default(),
        }
    }

    /// Get an ADM access token (from cache or request a new one)
    async fn get_access_token(&self) -> Result<String, RouterError> {
        let mut token_info = self.token_info.lock().await;

        if token_info.expiration_time > sec_since_epoch() + 60 {
            trace!("Using cached access token");
            return Ok(token_info.token.clone());
        }

        trace!("Access token is out of date, requesting a new one");
        let oauth_url = self.base_url.join("auth/O2/token").unwrap();
        let response = self
            .http
            .post(oauth_url)
            .form(&serde_json::json!({
                "grant_type": "client_credentials",
                "scope": "messaging:push",
                "client_id": &self.profile.client_id,
                "client_secret": &self.profile.client_secret,
            }))
            .send()
            .await
            .map_err(AdmError::Http)?;
        trace!("response = {:?}", response);

        if response.status() != 200 {
            let status = response.status();
            let error_response: AdmResponseError = response
                .json()
                .await
                .map_err(AdmError::DeserializeResponse)?;
            return Err(RouterError::Upstream {
                status: status.to_string(),
                message: error_response
                    .reason
                    .unwrap_or_else(|| "Unknown reason".to_string()),
            });
        }

        let token_response: TokenResponse = response
            .json()
            .await
            .map_err(AdmError::DeserializeResponse)?;
        token_info.token = token_response.access_token;
        token_info.expiration_time = sec_since_epoch() + token_response.expires_in;

        Ok(token_info.token.clone())
    }

    /// Send the message data to ADM. The device's current registration ID is
    /// returned. If it is different than the current stored ID, the stored ID
    /// should be updated.
    pub async fn send(
        &self,
        data: HashMap<&'static str, String>,
        registration_id: String,
        ttl: usize,
    ) -> Result<String, RouterError> {
        // Build the ADM message
        let message = serde_json::json!({
            "data": data,
            "expiresAfter": ttl,
        });
        let access_token = self.get_access_token().await?;
        let url = self
            .base_url
            .join(&format!(
                "messaging/registrations/{}/messages",
                registration_id
            ))
            .map_err(AdmError::ParseUrl)?;

        // Make the request
        let response = self
            .http
            .post(url)
            .header("Authorization", format!("Bearer {}", access_token.as_str()))
            .header("Accept", "application/json")
            .header(
                "X-Amzn-Type-Version",
                "com.amazon.device.messaging.ADMMessage@1.0",
            )
            .header(
                "X-Amzn-Accept-Type",
                "com.amazon.device.messaging.ADMSendResult@1.0",
            )
            .json(&message)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    RouterError::RequestTimeout
                } else {
                    RouterError::Connect(e)
                }
            })?;

        // Handle error
        let status = response.status();
        if status != 200 {
            let response_error: AdmResponseError = response
                .json()
                .await
                .map_err(AdmError::DeserializeResponse)?;

            return Err(match (status, response_error.reason) {
                (StatusCode::UNAUTHORIZED, _) => RouterError::Authentication,
                (StatusCode::NOT_FOUND, _) => RouterError::NotFound,
                (status, reason) => RouterError::Upstream {
                    status: status.to_string(),
                    message: reason.unwrap_or_else(|| "Unknown reason".to_string()),
                },
            });
        }

        let send_response: AdmSendResponse = response
            .json()
            .await
            .map_err(AdmError::DeserializeResponse)?;
        Ok(send_response.registration_id)
    }
}

#[cfg(test)]
pub mod tests {
    use crate::routers::adm::client::AdmClient;
    use crate::routers::adm::settings::{AdmProfile, AdmSettings};
    use crate::routers::RouterError;
    use std::collections::HashMap;
    use url::Url;

    pub const REGISTRATION_ID: &str = "test-registration-id";
    pub const CLIENT_ID: &str = "test-client-id";
    pub const CLIENT_SECRET: &str = "test-client-secret";
    const ACCESS_TOKEN: &str = "test-access-token";

    /// Mock the OAuth token endpoint to provide the access token
    pub fn mock_token_endpoint() -> mockito::Mock {
        mockito::mock("POST", "/auth/O2/token")
            .with_body(
                serde_json::json!({
                    "access_token": ACCESS_TOKEN,
                    "expires_in": 3600,
                    "scope": "messaging:push",
                    "token_type": "Bearer"
                })
                .to_string(),
            )
            .create()
    }

    /// Start building a mock for the ADM endpoint
    pub fn mock_adm_endpoint_builder() -> mockito::Mock {
        mockito::mock(
            "POST",
            format!("/messaging/registrations/{}/messages", REGISTRATION_ID).as_str(),
        )
    }

    /// Make an AdmClient which uses the mock server
    fn make_client() -> AdmClient {
        AdmClient::new(
            &AdmSettings {
                base_url: Url::parse(&mockito::server_url()).unwrap(),
                ..Default::default()
            },
            AdmProfile {
                client_id: CLIENT_ID.to_string(),
                client_secret: CLIENT_SECRET.to_string(),
            },
            reqwest::Client::new(),
        )
    }

    /// The ADM client uses the access token and parameters to build the
    /// expected request.
    #[tokio::test]
    async fn sends_correct_request() {
        let client = make_client();
        let _token_mock = mock_token_endpoint();
        let adm_mock = mock_adm_endpoint_builder()
            .match_header("Authorization", format!("Bearer {}", ACCESS_TOKEN).as_str())
            .match_header("Content-Type", "application/json")
            .match_header("Accept", "application/json")
            .match_header(
                "X-Amzn-Type-Version",
                "com.amazon.device.messaging.ADMMessage@1.0",
            )
            .match_header(
                "X-Amzn-Accept-Type",
                "com.amazon.device.messaging.ADMSendResult@1.0",
            )
            .match_body(r#"{"data":{"is_test":"true"},"expiresAfter":42}"#)
            .with_body(r#"{"registrationID":"test-registration-id2"}"#)
            .create();

        let mut data = HashMap::new();
        data.insert("is_test", "true".to_string());

        let result = client.send(data, REGISTRATION_ID.to_string(), 42).await;
        assert!(result.is_ok(), "result = {:?}", result);
        assert_eq!(result.unwrap(), "test-registration-id2");
        adm_mock.assert();
    }

    /// Authorization errors are handled
    #[tokio::test]
    async fn unauthorized() {
        let client = make_client();
        let _token_mock = mock_token_endpoint();
        let _adm_mock = mock_adm_endpoint_builder()
            .with_status(401)
            .with_body(r#"{"reason":"test-message"}"#)
            .create();

        let result = client
            .send(HashMap::new(), REGISTRATION_ID.to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), RouterError::Authentication),
            "result = {:?}",
            result
        );
    }

    /// 404 errors are handled
    #[tokio::test]
    async fn not_found() {
        let client = make_client();
        let _token_mock = mock_token_endpoint();
        let _adm_mock = mock_adm_endpoint_builder()
            .with_status(404)
            .with_body(r#"{"reason":"test-message"}"#)
            .create();

        let result = client
            .send(HashMap::new(), REGISTRATION_ID.to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(result.as_ref().unwrap_err(), RouterError::NotFound),
            "result = {:?}",
            result
        );
    }

    /// Unhandled errors (where a reason is returned) are wrapped and returned
    #[tokio::test]
    async fn other_adm_error() {
        let client = make_client();
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_adm_endpoint_builder()
            .with_status(400)
            .with_body(r#"{"reason":"test-message"}"#)
            .create();

        let result = client
            .send(HashMap::new(), REGISTRATION_ID.to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err(),
                RouterError::Upstream { status, message }
                    if status == "400 Bad Request" && message == "test-message"
            ),
            "result = {:?}",
            result
        );
    }

    /// Unknown errors (where a reason is NOT returned) are handled
    #[tokio::test]
    async fn unknown_adm_error() {
        let client = make_client();
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_adm_endpoint_builder()
            .with_status(400)
            .with_body("{}")
            .create();

        let result = client
            .send(HashMap::new(), REGISTRATION_ID.to_string(), 42)
            .await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err(),
                RouterError::Upstream { status, message }
                    if status == "400 Bad Request" && message == "Unknown reason"
            ),
            "result = {:?}",
            result
        );
    }
}
