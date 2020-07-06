use crate::server::routers::fcm::error::FcmError;
use crate::server::routers::fcm::settings::FcmCredential;
use reqwest::StatusCode;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;
use yup_oauth2::authenticator::DefaultAuthenticator;
use yup_oauth2::ServiceAccountAuthenticator;

/// The timeout for FCM requests, in seconds
const TIMEOUT: Duration = Duration::from_secs(3);
const OAUTH_SCOPES: &[&str] = &["https://www.googleapis.com/auth/firebase.messaging"];

/// Holds application-specific Firebase data and authentication. This client
/// handles sending notifications to Firebase.
pub struct FcmClient {
    pub endpoint: String,
    pub auth: DefaultAuthenticator,
    pub http: reqwest::Client,
}

impl FcmClient {
    /// Create an `FcmClient` using the provided credential
    pub async fn new(credential: FcmCredential, http: reqwest::Client) -> std::io::Result<Self> {
        let key_data = yup_oauth2::read_service_account_key(&credential.auth_file).await?;
        let auth = ServiceAccountAuthenticator::builder(key_data)
            .build()
            .await?;

        Ok(FcmClient {
            endpoint: format!(
                "https://fcm.googleapis.com/v1/projects/{}/messages:send",
                credential.project_id
            ),
            auth,
            http,
        })
    }

    /// Send the message data to FCM
    pub async fn send(
        &self,
        data: HashMap<&'static str, String>,
        token: String,
        ttl: usize,
    ) -> Result<(), FcmError> {
        // Build the FCM message
        let message = serde_json::json!({
            "message": {
                "token": token,
                "android": {
                    "ttl": format!("{}s", ttl),
                    "data": data
                }
            }
        });
        let access_token = self.auth.token(OAUTH_SCOPES).await?;

        // Make the request
        let response = self
            .http
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", access_token.as_str()))
            .header("Content-Type", "application/json; UTF-8")
            .body(message.to_string())
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(FcmError::FcmRequest)?;

        // Handle error
        if let Err(e) = response.error_for_status_ref() {
            let data: FcmResponse = response
                .json()
                .await
                .map_err(FcmError::DeserializeResponse)?;

            return Err(match (e.status(), data.error) {
                (Some(StatusCode::UNAUTHORIZED), _) => FcmError::FcmAuthentication,
                (Some(StatusCode::NOT_FOUND), _) => FcmError::FcmNotFound,
                (_, Some(error)) => FcmError::FcmUpstream {
                    status: error.status,
                    message: error.message,
                },
                _ => FcmError::FcmUnknown,
            });
        }

        Ok(())
    }
}

#[derive(Deserialize)]
struct FcmResponse {
    error: Option<FcmErrorResponse>,
}

#[derive(Deserialize)]
struct FcmErrorResponse {
    status: String,
    message: String,
}
