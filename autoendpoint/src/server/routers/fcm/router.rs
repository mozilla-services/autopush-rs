use crate::error::ApiResult;
use crate::server::extractors::notification::Notification;
use crate::server::routers::fcm::client::FcmClient;
use crate::server::routers::fcm::error::FcmError;
use crate::server::routers::fcm::settings::FcmCredential;
use crate::server::routers::{Router, RouterResponse};
use crate::server::FcmSettings;
use async_trait::async_trait;
use cadence::StatsdClient;
use std::collections::HashMap;

/// Firebase Cloud Messaging router
pub struct FcmRouter {
    settings: FcmSettings,
    metrics: StatsdClient,
    http: reqwest::Client,
    /// A map from application ID to an authenticated FCM client
    clients: HashMap<String, FcmClient>,
}

impl FcmRouter {
    /// Create a new `FcmRouter`
    pub async fn new(
        settings: FcmSettings,
        http: reqwest::Client,
        metrics: StatsdClient,
    ) -> Result<Self, FcmError> {
        let credentials = settings.credentials()?;
        let clients = Self::create_clients(credentials, http.clone())
            .await
            .map_err(FcmError::OAuthClientBuild)?;

        Ok(Self {
            settings,
            metrics,
            http,
            clients,
        })
    }

    /// Create FCM clients for each application
    async fn create_clients(
        credentials: HashMap<String, FcmCredential>,
        http: reqwest::Client,
    ) -> std::io::Result<HashMap<String, FcmClient>> {
        let mut clients = HashMap::new();

        for (profile, credential) in credentials {
            clients.insert(profile, FcmClient::new(credential, http.clone()).await?);
        }

        Ok(clients)
    }
}

#[async_trait(?Send)]
impl Router for FcmRouter {
    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        unimplemented!()
    }
}
