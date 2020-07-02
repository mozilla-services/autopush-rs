use crate::error::ApiResult;
use crate::server::extractors::notification::Notification;
use crate::server::routers::fcm::client::FcmClient;
use crate::server::routers::fcm::error::FcmError;
use crate::server::routers::fcm::settings::FcmCredential;
use crate::server::routers::{Router, RouterResponse};
use crate::server::FcmSettings;
use actix_web::http::StatusCode;
use async_trait::async_trait;
use autopush_common::util::InsertOpt;
use cadence::{Counted, StatsdClient};
use serde_json::Value;
use std::collections::HashMap;
use url::Url;

const MAX_TTL: usize = 2419200;

/// Firebase Cloud Messaging router
pub struct FcmRouter {
    settings: FcmSettings,
    endpoint_url: Url,
    metrics: StatsdClient,
    /// A map from application ID to an authenticated FCM client
    clients: HashMap<String, FcmClient>,
}

impl FcmRouter {
    /// Create a new `FcmRouter`
    pub async fn new(
        settings: FcmSettings,
        endpoint_url: Url,
        http: reqwest::Client,
        metrics: StatsdClient,
    ) -> Result<Self, FcmError> {
        let credentials = settings.credentials()?;
        let clients = Self::create_clients(credentials, http.clone())
            .await
            .map_err(FcmError::OAuthClientBuild)?;

        Ok(Self {
            settings,
            endpoint_url,
            metrics,
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

    /// Convert a notification into a WebPush message
    fn build_message_data(
        &self,
        notification: &Notification,
    ) -> ApiResult<HashMap<&'static str, String>> {
        let mut message_data = HashMap::new();
        message_data.insert(
            "chid",
            notification
                .subscription
                .channel_id
                .to_simple_ref()
                .to_string(),
        );

        // Only add the other headers if there's data
        if let Some(data) = &notification.data {
            if data.len() > self.settings.max_data {
                // Too much data. Tell the client how many bytes extra they had.
                return Err(FcmError::TooMuchData(data.len() - self.settings.max_data).into());
            }

            // Add the body and headers
            message_data.insert("body", data.clone());
            message_data.insert_opt("con", notification.headers.encoding.as_ref());
            message_data.insert_opt("enc", notification.headers.encryption.as_ref());
            message_data.insert_opt("cryptokey", notification.headers.crypto_key.as_ref());
            message_data.insert_opt("enckey", notification.headers.encryption_key.as_ref());
        }

        Ok(message_data)
    }
}

#[async_trait(?Send)]
impl Router for FcmRouter {
    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        let router_data = &notification.subscription.user.router_data;
        let fcm_token = router_data
            .get("token")
            .and_then(Value::as_str)
            .ok_or(FcmError::NoRegistrationToken)?;
        let app_id = router_data
            .get("app_id")
            .and_then(Value::as_str)
            .ok_or(FcmError::NoAppId)?;
        let ttl = MAX_TTL.min(self.settings.ttl.max(notification.headers.ttl as usize));
        let message_data = self.build_message_data(notification)?;

        // Send the notification to FCM
        let client = self.clients.get(app_id).ok_or(FcmError::InvalidAppId)?;
        client
            .send(message_data, fcm_token.to_string(), ttl)
            .await?;

        // Sent successfully, update metrics and make response
        self.metrics
            .incr_with_tags("notification.bridge.sent")
            .with_tag("platform", "fcmv1")
            .send();
        self.metrics
            .count_with_tags(
                "notification.message_data",
                notification.data.as_ref().map(String::len).unwrap_or(0) as i64,
            )
            .with_tag("platform", "fcmv1")
            .with_tag("destination", "Direct")
            .send();

        Ok(RouterResponse {
            status: StatusCode::OK,
            headers: {
                let mut map = HashMap::new();
                map.insert(
                    "Location",
                    format!("{}/m/{}", self.endpoint_url, notification.message_id),
                );
                map.insert("TTL", notification.headers.ttl.to_string());
                map
            },
            body: None,
        })
    }
}
