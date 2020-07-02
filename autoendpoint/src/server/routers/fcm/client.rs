use crate::error::ApiResult;
use crate::server::routers::fcm::settings::FcmCredential;
use std::collections::HashMap;
use yup_oauth2::authenticator::DefaultAuthenticator;
use yup_oauth2::ServiceAccountAuthenticator;

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
    ) -> ApiResult<()> {
        unimplemented!()
    }
}
