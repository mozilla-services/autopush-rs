use super::client::WnsClient;
use super::error::WnsError;
use super::settings::{WnsChannelSettings, WnsSettings};
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::common::{build_message_data, handle_error, incr_success_metrics};
use crate::routers::wns::client::WnsAccessToken;
use crate::routers::{Router, RouterError, RouterResponse};

use async_trait::async_trait;
use cadence::StatsdClient;
use chrono::Utc;

use autopush_common::db::client::DbClient;
use autopush_common::reliability::{PushReliability, ReliabilityState};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// ##Windows Notification System (WNS) router
///
/// This is a very simple implementation done to research the WNS protocol. It needs
/// tests, validaiton, and a lot of other work before it sees the light of day.
///  
/// WNS requires two tokens. One is the server access token, which expires regularly, and
/// makes up the authentication header for the send request. The second is the channel
/// token, which is provided by the UA during registration. This is added to the
/// send URI.
///
pub struct WnsRouter {
    /// The parsed server settings.
    server_settings: WnsSettings,
    /// A map from application ID to an WNS client
    clients: HashMap<String, WnsClient>,
    /// The endpoint URL for this router, used for constructing the response.
    endpoint_url: url::Url,
    metrics: Arc<StatsdClient>,
    db: Box<dyn DbClient>,
    /// Set of AppID specific access tokens.
    access_tokens: Arc<RwLock<HashMap<String, WnsAccessToken>>>,
    #[cfg(feature = "reliable_report")]
    reliability: Arc<PushReliability>,
}

impl WnsRouter {
    /// Create a new `WnsRouter` from the raw settings information.
    pub fn new(
        settings: &WnsSettings,
        endpoint_url: url::Url,
        http: reqwest::Client,
        metrics: Arc<StatsdClient>,
        db: Box<dyn DbClient>,
        access_tokens: Arc<RwLock<HashMap<String, WnsAccessToken>>>,
        #[cfg(feature = "reliable_report")] reliability: Arc<PushReliability>,
    ) -> Result<Self, WnsError> {
        let clients = Self::create_clients(&settings.channels, &http)?;
        Ok(Self {
            server_settings: settings.clone(),
            endpoint_url,
            clients,
            metrics,
            db,
            access_tokens,
            #[cfg(feature = "reliable_report")]
            reliability,
        })
    }

    /// There can be multiple mozilla app_ids that could be set to different WNS creds.
    ///
    fn create_clients(
        server_settings: &HashMap<String, WnsChannelSettings>,
        http: &reqwest::Client,
    ) -> Result<HashMap<String, WnsClient>, WnsError> {
        let mut clients = HashMap::new();
        for (app_id, profile) in server_settings {
            clients.insert(
                app_id.to_lowercase(),
                WnsClient::new(profile, http.clone())?,
            );
        }
        Ok(clients)
    }
}

#[async_trait(?Send)]
impl Router for WnsRouter {
    /// Register the user token for WNS. We can't really check to see if the token is valid
    /// at this point, so presume it is.
    fn register(
        &self,
        router_data_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, Value>, RouterError> {
        if !self.clients.contains_key(app_id) {
            return Err(WnsError::General(format!("Unknown App ID: {}", app_id.to_owned())).into());
        }
        let mut router_data = HashMap::new();
        router_data.insert("app_id".to_string(), Value::String(app_id.to_string()));
        router_data.insert(
            "channel_token".to_string(),
            Value::String(router_data_input.token.clone()),
        );

        Ok(router_data)
    }

    async fn route_notification(
        &self,
        mut notification: Notification,
    ) -> ApiResult<RouterResponse> {
        debug!(
            "Sending Test notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(WnsError::NoRouterData)?;
        let app_id = router_data
            .get("app_id")
            .and_then(|value: &Value| value.as_str())
            .ok_or(WnsError::NoAppId)?
            .to_string();
        let channel_token = router_data
            .get("channel_token")
            .and_then(|value| value.as_str())
            .ok_or(WnsError::NoRegistrationToken)?
            .to_string();
        // Send the notification to WNS
        let client = self
            .clients
            .get(&app_id)
            .ok_or_else(|| WnsError::InvalidAppId(app_id.clone()))?
            .clone();

        // TODO: Check to see if you only get one access token, or if fetching a new
        // token invalidates the old one.
        // TODO: This can probably be collapsed using maps and filter, but I wanted to understand what's going on.
        let mut access_token: Option<WnsAccessToken> =
            if let Ok(access_tokens) = self.access_tokens.read() {
                // get one of the stored tokens.
                if let Some(token) = access_tokens.get(&app_id) {
                    // Check if the token is expired.
                    if token.expires_on > Utc::now().timestamp() as u64 {
                        Some(token.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                return Err(
                    WnsError::General("Failed to acquire access token lock".to_string()).into(),
                );
            };

        if access_token.is_none() {
            // if there was no token, or the token was expired, fetch a new one.
            let new_token = match client.get_access_token(&app_id).await {
                Ok(token) => token,
                Err(e) => {
                    return Err(WnsError::General(format!(
                        "Could not fetch new access token: {}",
                        e
                    ))
                    .into());
                }
            };
            if let Ok(mut access_tokens) = self.access_tokens.write() {
                access_tokens.insert(app_id.clone(), new_token.clone());
            } else {
                return Err(
                    WnsError::General("Failed to acquire access token lock".to_string()).into(),
                );
            };
            access_token = Some(new_token);
        }

        let ttl = (self.server_settings.max_ttl).min(notification.headers.ttl as u64);
        let message_data = build_message_data(&notification)?;
        let platform = "wnsv1";
        trace!("Sending message to {platform}: [{:?}]", &app_id);
        // We can call unwrap here because we should already have a realized token.
        let access_token = access_token.unwrap().access_token;
        if let Err(e) = client
            .send(message_data, &channel_token, &access_token, ttl)
            .await
        {
            #[cfg(feature = "reliable_report")]
            notification
                .record_reliability(&self.reliability, ReliabilityState::Errored)
                .await;
            return Err(handle_error(
                e.into(),
                &self.metrics,
                self.db.as_ref(),
                platform,
                &app_id,
                notification.subscription.user.uaid,
                notification.subscription.vapid.clone(),
            )
            .await);
        };
        incr_success_metrics(&self.metrics, platform, &app_id, &notification);
        #[cfg(feature = "reliable_report")]
        // Record that we've sent the message out to WNS.
        // We can't set the state here because the notification isn't
        // mutable, but we are also essentially consuming the
        // notification nothing else should modify it.
        notification
            .record_reliability(&self.reliability, ReliabilityState::BridgeTransmitted)
            .await;
        // Sent successfully, update metrics and make response
        trace!("Send request was successful");

        Ok(RouterResponse::success(
            self.endpoint_url
                .join(&format!("/m/{}", notification.message_id))
                .expect("Message ID is not URL-safe")
                .to_string(),
            notification.headers.ttl as usize,
        ))
    }
}
