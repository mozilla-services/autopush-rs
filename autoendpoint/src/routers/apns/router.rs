use crate::error::{ApiError, ApiResult};
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::apns::error::ApnsError;
use crate::routers::apns::settings::{ApnsChannel, ApnsSettings};
use crate::routers::common::build_message_data;
use crate::routers::{Router, RouterError, RouterResponse};
use a2::request::notification::LocalizedAlert;
use a2::request::payload::{APSAlert, Payload, APS};
use a2::{Endpoint, NotificationOptions, Priority};
use async_trait::async_trait;
use cadence::{Counted, StatsdClient};
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use url::Url;

pub struct ApnsRouter {
    /// A map from release channel to APNS client
    clients: HashMap<String, ApnsClient>,
    settings: ApnsSettings,
    endpoint_url: Url,
    metrics: StatsdClient,
}

struct ApnsClient {
    client: a2::Client,
    topic: String,
}

impl ApnsRouter {
    /// Create a new APNS router. APNS clients will be initialized for each
    /// channel listed in the settings.
    pub async fn new(
        settings: ApnsSettings,
        endpoint_url: Url,
        metrics: StatsdClient,
    ) -> Result<Self, ApnsError> {
        let channels = settings.channels()?;

        let clients = futures::stream::iter(channels)
            .then(|(name, settings)| Self::create_client(name, settings))
            .try_collect()
            .await?;

        Ok(Self {
            clients,
            settings,
            endpoint_url,
            metrics,
        })
    }

    /// Create an APNS client for the channel
    async fn create_client(
        name: String,
        settings: ApnsChannel,
    ) -> Result<(String, ApnsClient), ApnsError> {
        let endpoint = if settings.sandbox {
            Endpoint::Sandbox
        } else {
            Endpoint::Production
        };
        let cert = tokio::fs::read(settings.cert).await?;
        let key = tokio::fs::read(settings.key).await?;
        let client = ApnsClient {
            client: a2::Client::certificate_parts(&cert, &key, endpoint)
                .map_err(ApnsError::ApnsClient)?,
            topic: settings
                .topic
                .unwrap_or_else(|| format!("com.mozilla.org.{}", name)),
        };

        Ok((name, client))
    }

    /// Update metrics after successfully routing the notification
    fn incr_success_metrics(&self, notification: &Notification, channel: &str) {
        self.metrics
            .incr_with_tags("notification.bridge.sent")
            .with_tag("platform", "apns")
            .with_tag("application", channel)
            .send();
        self.metrics
            .incr_with_tags(&format!("updates.client.bridge.apns.{}.sent", channel))
            .with_tag("platform", "apns")
            .send();
        self.metrics
            .count_with_tags(
                "notification.message_data",
                notification.data.as_ref().map(String::len).unwrap_or(0) as i64,
            )
            .with_tag("platform", "apns")
            .with_tag("destination", "Direct")
            .send();
    }

    /// Increment an error metric with some details
    fn incr_error_metric(&self, reason: &str, channel: &str) {
        self.metrics
            .incr_with_tags("notification.bridge.connection.error")
            .with_tag("platform", "apns")
            .with_tag("application", channel)
            .with_tag("reason", reason)
            .send()
    }

    /// Handle an error by logging, updating metrics, etc
    fn handle_error(&self, error: a2::Error, channel: &str) -> ApiError {
        match &error {
            a2::Error::ResponseError(response) => {
                if response.code == 410 {
                    debug!("APNS recipient has been unregistered");
                    self.incr_error_metric("unregistered", channel);
                    return ApiError::from(ApnsError::Unregistered);
                } else {
                    warn!("APNS error: {:?}", response.error);
                    self.incr_error_metric("response_error", channel);
                }
            }
            a2::Error::ConnectionError => {
                error!("APNS connection error");
                self.incr_error_metric("connection", channel);
            }
            _ => {
                warn!("Unknown error while sending APNS request: {}", error);
                self.incr_error_metric("unknown", channel);
            }
        }

        ApiError::from(ApnsError::ApnsUpstream(error))
    }
}

#[async_trait(?Send)]
impl Router for ApnsRouter {
    fn register(
        &self,
        router_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, Value>, RouterError> {
        if !self.clients.contains_key(app_id) {
            return Err(ApnsError::InvalidReleaseChannel.into());
        }

        let mut router_data = HashMap::new();
        router_data.insert(
            "token".to_string(),
            serde_json::to_value(&router_input.token).unwrap(),
        );
        router_data.insert(
            "rel_channel".to_string(),
            serde_json::to_value(app_id).unwrap(),
        );

        if let Some(aps) = &router_input.aps {
            if APS::deserialize(aps).is_err() {
                return Err(ApnsError::InvalidApsData.into());
            }

            router_data.insert("aps".to_string(), aps.clone());
        }

        Ok(router_data)
    }

    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        debug!(
            "Sending APNS notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        // Build message data
        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(ApnsError::NoDeviceToken)?;
        let token = router_data
            .get("token")
            .and_then(Value::as_str)
            .ok_or(ApnsError::NoDeviceToken)?;
        let channel = router_data
            .get("rel_channel")
            .and_then(Value::as_str)
            .ok_or(ApnsError::NoReleaseChannel)?;
        let aps: APS<'_> = router_data
            .get("aps")
            .and_then(|value| APS::deserialize(value).ok())
            .unwrap_or_else(|| APS {
                alert: Some(APSAlert::Localized({
                    LocalizedAlert {
                        title_loc_key: Some("SentTab.NoTabArrivingNotification.title"),
                        loc_key: Some("SentTab.NoTabArrivingNotification.body"),
                        ..Default::default()
                    }
                })),
                mutable_content: Some(1),
                ..Default::default()
            });
        let mut message_data = build_message_data(notification, self.settings.max_data)?;
        message_data.insert("ver", notification.message_id.clone());

        // Get client and build payload
        let ApnsClient { client, topic } = self.clients.get(channel).unwrap();
        let payload = Payload {
            aps,
            data: message_data
                .into_iter()
                .map(|(k, v)| (k, Value::String(v)))
                .collect(),
            device_token: token,
            options: NotificationOptions {
                apns_id: None,
                apns_priority: Some(Priority::High),
                apns_topic: Some(topic),
                apns_collapse_id: None,
                apns_expiration: None,
            },
        };

        // Send to APNS
        trace!("Sending message to APNS: {:?}", payload);
        if let Err(e) = client.send(payload).await {
            return Err(self.handle_error(e, channel));
        }

        // Sent successfully, update metrics and make response
        trace!("APNS request was successful");
        self.incr_success_metrics(notification, channel);

        Ok(RouterResponse::success(
            self.endpoint_url
                .join(&format!("/m/{}", notification.message_id))
                .expect("Message ID is not URL-safe")
                .to_string(),
            notification.headers.ttl as usize,
        ))
    }
}
