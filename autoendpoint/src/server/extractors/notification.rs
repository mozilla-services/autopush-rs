use crate::error::{ApiError, ApiErrorKind};
use crate::server::extractors::notification_headers::NotificationHeaders;
use crate::server::extractors::subscription::Subscription;
use crate::server::ServerState;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use autopush_common::util::sec_since_epoch;
use cadence::Counted;
use fernet::MultiFernet;
use futures::{future, FutureExt, StreamExt};
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use uuid::Uuid;

/// Extracts notification data from `Subscription` and request data
pub struct Notification {
    pub message_id: String,
    pub subscription: Subscription,
    pub headers: NotificationHeaders,
    pub timestamp: u64,
    pub data: Option<String>,
}

impl FromRequest for Notification {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, payload: &mut Payload<PayloadStream>) -> Self::Future {
        let req = req.clone();
        let mut payload = payload.take();

        async move {
            let subscription = Subscription::extract(&req).await?;
            let state = Data::<ServerState>::extract(&req)
                .await
                .expect("No server state found");

            // Read data
            let mut data = Vec::new();
            while let Some(item) = payload.next().await {
                data.extend_from_slice(&item.map_err(ApiErrorKind::PayloadError)?);

                // Make sure the payload isn't too big
                let max_bytes = state.settings.max_data_bytes;
                if data.len() > max_bytes {
                    return Err(ApiErrorKind::PayloadTooLarge(max_bytes).into());
                }
            }

            // Convert data to base64
            let data = if data.is_empty() {
                None
            } else {
                Some(base64::encode_config(data, base64::URL_SAFE_NO_PAD))
            };

            let headers = NotificationHeaders::from_request(&req, data.is_some())?;
            let timestamp = sec_since_epoch();
            let message_id = Self::generate_message_id(
                &state.fernet,
                &subscription.user.uaid,
                &subscription.channel_id,
                headers.topic.as_deref(),
                timestamp,
            );

            // Record the encoding if we have an encrypted payload
            if let Some(encoding) = &headers.content_encoding {
                if data.is_some() {
                    state
                        .metrics
                        .incr(&format!("updates.notification.encoding.{}", encoding))
                        .ok();
                }
            }

            Ok(Notification {
                message_id,
                subscription,
                headers,
                timestamp,
                data,
            })
        }
        .boxed_local()
    }
}

impl Serialize for Notification {
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(7))?;
        map.serialize_entry("channelID", &self.subscription.channel_id)?;
        map.serialize_entry("version", &self.message_id)?;
        map.serialize_entry("ttl", &self.headers.ttl)?;
        map.serialize_entry("topic", &self.headers.topic)?;
        map.serialize_entry("timestamp", &self.timestamp)?;
        map.serialize_entry("data", &self.data)?;
        map.serialize_entry("headers", &self.headers)?;
        map.end()
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
        uaid: &Uuid,
        channel_id: &Uuid,
        topic: Option<&str>,
        timestamp: u64,
    ) -> String {
        let message_id = if let Some(topic) = topic {
            format!(
                "01:{}:{}:{}",
                uaid.to_simple_ref(),
                channel_id.to_simple_ref(),
                topic
            )
        } else {
            format!(
                "02:{}:{}:{}",
                uaid.to_simple_ref(),
                channel_id.to_simple_ref(),
                timestamp
            )
        };

        fernet.encrypt(message_id.as_bytes())
    }
}
