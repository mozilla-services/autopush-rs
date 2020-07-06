use crate::error::{ApiError, ApiResult};
use crate::server::extractors::notification::Notification;
use crate::server::routers::fcm::client::FcmClient;
use crate::server::routers::fcm::error::FcmError;
use crate::server::routers::fcm::settings::FcmCredential;
use crate::server::routers::{Router, RouterResponse};
use crate::server::FcmSettings;
use async_trait::async_trait;
use autopush_common::util::InsertOpt;
use cadence::{Counted, StatsdClient};
use serde_json::Value;
use std::collections::HashMap;
use url::Url;

/// 28 days
const MAX_TTL: usize = 28 * 24 * 60 * 60;

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

    /// Handle an error by logging, updating metrics, etc
    fn handle_error(&self, error: FcmError) -> ApiError {
        match &error {
            FcmError::FcmAuthentication => {
                error!("FCM authentication error");
                self.incr_error_metric("authentication");
            }
            FcmError::FcmRequestTimeout => {
                warn!("FCM timeout");
                self.incr_error_metric("timeout");
            }
            FcmError::FcmConnect(e) => {
                warn!("FCM unavailable: {error}", error = e.to_string());
                self.incr_error_metric("connection_unavailable");
            }
            FcmError::FcmNotFound => {
                debug!("FCM recipient not found");
                self.incr_error_metric("recipient_gone");
            }
            FcmError::FcmUpstream { .. } | FcmError::FcmUnknown => {
                warn!("FCM error: {error}", error = error.to_string());
                self.incr_error_metric("server_error");
            }
            _ => {
                warn!(
                    "Unknown error while sending FCM request: {error}",
                    error = error.to_string()
                );
                self.incr_error_metric("unknown");
            }
        }

        ApiError::from(error)
    }

    /// Update metrics after successfully routing the notification
    fn incr_success_metrics(&self, notification: &Notification) {
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
    }

    /// Increment `notification.bridge.error` with a reason
    fn incr_error_metric(&self, reason: &str) {
        self.metrics
            .incr_with_tags("notification.bridge.error")
            .with_tag("platform", "fcmv1")
            .with_tag("reason", reason)
            .send();
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
        if let Err(e) = client.send(message_data, fcm_token.to_string(), ttl).await {
            return Err(self.handle_error(e));
        }

        // Sent successfully, update metrics and make response
        self.incr_success_metrics(notification);

        Ok(RouterResponse::success(
            format!("{}/m/{}", self.endpoint_url, notification.message_id),
            notification.headers.ttl as usize,
        ))
    }
}
