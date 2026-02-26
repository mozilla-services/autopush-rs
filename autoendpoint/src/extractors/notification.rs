use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::extractors::{
    message_id::MessageId, notification_headers::NotificationHeaders, subscription::Subscription,
};
use crate::server::AppState;
use actix_web::{dev::Payload, web, FromRequest, HttpRequest};
use autopush_common::util::{b64_encode_url, ms_since_epoch, sec_since_epoch};
use cadence::CountedExt;
use fernet::MultiFernet;
use futures::{future, FutureExt};
use serde::Serialize;
use std::collections::HashMap;
use uuid::Uuid;

/// Wire format for delivering notifications to connection servers.
/// Uses a single serialization pass instead of building a HashMap of serde_json::Values.
#[derive(Debug, Serialize)]
pub struct NotificationForDelivery<'a> {
    #[serde(rename = "channelID")]
    pub channel_id: uuid::Uuid,
    pub version: &'a str,
    pub ttl: i64,
    pub topic: Option<&'a str>,
    pub timestamp: u64,
    #[cfg(feature = "reliable_report")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliability_id: Option<&'a str>,
    #[cfg(feature = "reliable_report")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliable_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
}

/// Extracts notification data from `Subscription` and request data
#[derive(Clone, Debug)]
pub struct Notification {
    /// Unique message_id for this notification
    pub message_id: String,
    /// The subscription information block
    pub subscription: Subscription,
    /// Set of associated crypto headers
    pub headers: NotificationHeaders,
    /// UNIX timestamp in seconds
    pub timestamp: u64,
    /// UNIX timestamp in milliseconds
    pub sort_key_timestamp: u64,
    /// The encrypted notification body
    pub data: Option<String>,
    #[cfg(feature = "reliable_report")]
    /// The current state the message was in (if tracked)
    pub reliable_state: Option<autopush_common::reliability::ReliabilityState>,
    #[cfg(feature = "reliable_report")]
    pub reliability_id: Option<String>,
}

impl FromRequest for Notification {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let req = req.clone();
        let mut payload = payload.take();

        async move {
            let subscription = Subscription::extract(&req).await?;
            let app_state = web::Data::<AppState>::extract(&req)
                .await
                .expect("No server state found");

            let max_notification_ttl_secs = app_state.settings.max_notification_ttl;
            // Read data
            let data = web::Bytes::from_request(&req, &mut payload)
                .await
                .map_err(|e| {
                    debug!("▶▶ Request read payload error: {:?}", &e);
                    ApiErrorKind::PayloadError(e)
                })?;

            // Convert data to base64
            let data = if data.is_empty() {
                None
            } else {
                Some(b64_encode_url(&data.to_vec()))
            };

            let headers =
                NotificationHeaders::from_request(&req, data.is_some(), max_notification_ttl_secs)?;
            let timestamp = sec_since_epoch();
            let sort_key_timestamp = ms_since_epoch();
            let message_id = Self::generate_message_id(
                &app_state.fernet,
                subscription.user.uaid,
                subscription.channel_id,
                headers.topic.as_deref(),
                sort_key_timestamp,
            );

            #[cfg(feature = "reliable_report")]
            let reliability_id = subscription.reliability_id.clone();

            #[allow(unused_mut)]
            let mut notif = Notification {
                message_id,
                subscription,
                headers,
                timestamp,
                sort_key_timestamp,
                data,
                #[cfg(feature = "reliable_report")]
                reliable_state: None,
                #[cfg(feature = "reliable_report")]
                reliability_id,
            };

            #[cfg(feature = "reliable_report")]
            // Brand new notification, so record it as "Received"
            notif
                .record_reliability(
                    &app_state.reliability,
                    autopush_common::reliability::ReliabilityState::Received,
                )
                .await;

            // Record the encoding if we have an encrypted payload
            if let Some(encoding) = &notif.headers.encoding {
                if notif.data.is_some() {
                    app_state
                        .metrics
                        .incr(&format!("updates.notification.encoding.{encoding}"))
                        .ok();
                }
            }

            Ok(notif)
        }
        .boxed_local()
    }
}

impl From<Notification> for autopush_common::notification::Notification {
    fn from(notification: Notification) -> Self {
        let topic = notification.headers.topic.clone();
        let sortkey_timestamp = topic.is_none().then_some(notification.sort_key_timestamp);
        autopush_common::notification::Notification {
            channel_id: notification.subscription.channel_id,
            version: notification.message_id,
            ttl: notification.headers.ttl as u64,
            topic,
            timestamp: notification.timestamp,
            data: notification.data,
            sortkey_timestamp,
            #[cfg(feature = "reliable_report")]
            reliability_id: notification.subscription.reliability_id,
            headers: {
                let headers: HashMap<String, String> = notification.headers.into();
                if headers.is_empty() {
                    None
                } else {
                    Some(headers)
                }
            },
            #[cfg(feature = "reliable_report")]
            reliable_state: notification.reliable_state,
        }
    }
}

impl Notification {
    /// Generate a message-id suitable for accessing the message
    ///
    /// For topic messages, a sort_key version of 01 is used, and the topic
    /// is included for reference:
    ///
    ///     Encrypted('01' : uaid.hex : channel_id.hex : topic)
    ///
    /// For non-topic messages, a sort_key version of 02 is used:
    ///
    ///     Encrypted('02' : uaid.hex : channel_id.hex : timestamp)
    fn generate_message_id(
        fernet: &MultiFernet,
        uaid: Uuid,
        channel_id: Uuid,
        topic: Option<&str>,
        timestamp: u64,
    ) -> String {
        let message_id = if let Some(topic) = topic {
            MessageId::WithTopic {
                uaid,
                channel_id,
                topic: topic.to_string(),
            }
        } else {
            MessageId::WithoutTopic {
                uaid,
                channel_id,
                timestamp,
            }
        };

        message_id.encrypt(fernet)
    }

    pub fn has_topic(&self) -> bool {
        self.headers.topic.is_some()
    }

    /// Convert to a common Notification by borrowing, avoiding a full clone
    /// of the Subscription (which contains User with router_data HashMap).
    pub fn to_common_notification(&self) -> autopush_common::notification::Notification {
        let topic = self.headers.topic.clone();
        let sortkey_timestamp = topic.is_none().then_some(self.sort_key_timestamp);
        autopush_common::notification::Notification {
            channel_id: self.subscription.channel_id,
            version: self.message_id.clone(),
            ttl: self.headers.ttl as u64,
            topic,
            timestamp: self.timestamp,
            data: self.data.clone(),
            sortkey_timestamp,
            #[cfg(feature = "reliable_report")]
            reliability_id: self.subscription.reliability_id.clone(),
            headers: {
                let headers: HashMap<String, String> = self.headers.clone().into();
                if headers.is_empty() {
                    None
                } else {
                    Some(headers)
                }
            },
            #[cfg(feature = "reliable_report")]
            reliable_state: self.reliable_state,
        }
    }

    /// Serialize the notification for delivery to the connection server. Some
    /// fields in `autopush_common`'s `Notification` are marked with
    /// `#[serde(skip_serializing)]` so they are not shown to the UA. These
    /// fields are still required when delivering to the connection server, so
    /// we can't simply convert this notification type to that one and serialize
    /// via serde.
    pub fn serialize_for_delivery(&self) -> ApiResult<NotificationForDelivery<'_>> {
        let headers = self.data.as_ref().map(|_| {
            let h: HashMap<String, String> = self.headers.clone().into();
            h
        });
        Ok(NotificationForDelivery {
            channel_id: self.subscription.channel_id,
            version: &self.message_id,
            ttl: self.headers.ttl,
            topic: self.headers.topic.as_deref(),
            timestamp: self.timestamp,
            #[cfg(feature = "reliable_report")]
            reliability_id: self.subscription.reliability_id.as_deref(),
            #[cfg(feature = "reliable_report")]
            reliable_state: self.reliable_state.map(|s| s.to_string()),
            data: self.data.as_deref(),
            headers,
        })
    }

    #[cfg(feature = "reliable_report")]
    pub async fn record_reliability(
        &mut self,
        reliability: &autopush_common::reliability::PushReliability,
        state: autopush_common::reliability::ReliabilityState,
    ) {
        self.reliable_state = reliability
            .record(
                &self.reliability_id,
                state,
                &self.reliable_state,
                Some(self.timestamp + self.headers.ttl as u64),
            )
            .await
            .inspect_err(|e| {
                warn!("🔍⚠️ Unable to record reliability state log: {:?}", e);
            })
            .unwrap_or(Some(state))
    }
}
