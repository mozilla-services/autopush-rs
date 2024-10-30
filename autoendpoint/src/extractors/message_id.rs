use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::server::AppState;
use actix_web::dev::Payload;
use actix_web::{web::Data, FromRequest, HttpRequest};
use autopush_common::notification::{STANDARD_NOTIFICATION_PREFIX, TOPIC_NOTIFICATION_PREFIX};
use fernet::MultiFernet;
use futures::future;
use uuid::Uuid;

/// Holds information about a notification. The information is encoded and
/// encrypted into a "message ID" which is presented to the user. Later, the
/// user can send us the message ID to perform operations on the associated
/// notification (e.g. delete it).
#[derive(Debug)]
pub enum MessageId {
    WithTopic {
        uaid: Uuid,
        channel_id: Uuid,
        topic: String,
    },
    WithoutTopic {
        uaid: Uuid,
        channel_id: Uuid,
        timestamp_ms: u64,
    },
}

impl FromRequest for MessageId {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let message_id_param = req
            .match_info()
            .get("message_id")
            .expect("{message_id} must be part of the path");
        let app_state: Data<AppState> = Data::extract(req)
            .into_inner()
            .expect("No server state found");

        future::ready(MessageId::decrypt(&app_state.fernet, message_id_param))
    }
}

impl MessageId {
    /// Encode and encrypt the message ID
    pub fn encrypt(&self, fernet: &MultiFernet) -> String {
        let id_str = match self {
            MessageId::WithTopic {
                uaid,
                channel_id,
                topic,
            } => format!(
                "{}:{}:{}:{}",
                TOPIC_NOTIFICATION_PREFIX,
                &uaid.as_simple(),
                &channel_id.as_simple(),
                topic
            ),
            MessageId::WithoutTopic {
                uaid,
                channel_id,
                timestamp_ms,
            } => format!(
                "{}:{}:{}:{}",
                STANDARD_NOTIFICATION_PREFIX,
                uaid.as_simple(),
                channel_id.as_simple(),
                timestamp_ms
            ),
        };

        fernet.encrypt(id_str.as_bytes())
    }

    /// Decrypt and decode the message ID
    pub fn decrypt(fernet: &MultiFernet, message_id: &str) -> ApiResult<Self> {
        let decrypted_bytes = fernet
            .decrypt(message_id)
            .map_err(|_| ApiErrorKind::InvalidMessageId)?;
        let decrypted_str = String::from_utf8_lossy(&decrypted_bytes);
        let segments: Vec<_> = decrypted_str.split(':').collect();

        if segments.len() != 4 {
            return Err(ApiErrorKind::InvalidMessageId.into());
        }

        let (version, uaid, chid, topic_or_timestamp) =
            (segments[0], segments[1], segments[2], segments[3]);

        match version {
            "01" => Ok(MessageId::WithTopic {
                uaid: Uuid::parse_str(uaid).map_err(|_| ApiErrorKind::InvalidMessageId)?,
                channel_id: Uuid::parse_str(chid).map_err(|_| ApiErrorKind::InvalidMessageId)?,
                topic: topic_or_timestamp.to_string(),
            }),
            "02" => Ok(MessageId::WithoutTopic {
                uaid: Uuid::parse_str(uaid).map_err(|_| ApiErrorKind::InvalidMessageId)?,
                channel_id: Uuid::parse_str(chid).map_err(|_| ApiErrorKind::InvalidMessageId)?,
                timestamp_ms: topic_or_timestamp
                    .parse()
                    .map_err(|_| ApiErrorKind::InvalidMessageId)?,
            }),
            _ => Err(ApiErrorKind::InvalidMessageId.into()),
        }
    }

    /// Get the UAID of the associated notification
    pub fn uaid(&self) -> Uuid {
        match self {
            MessageId::WithTopic { uaid, .. } => *uaid,
            MessageId::WithoutTopic { uaid, .. } => *uaid,
        }
    }

    /// Get the sort-key for the associated notification
    pub fn sort_key(&self) -> String {
        match self {
            MessageId::WithTopic {
                channel_id, topic, ..
            } => format!(
                "{}:{}:{}",
                TOPIC_NOTIFICATION_PREFIX,
                channel_id.as_hyphenated(),
                topic
            ),
            MessageId::WithoutTopic {
                channel_id,
                timestamp_ms,
                ..
            } => format!(
                "{}:{}:{}",
                STANDARD_NOTIFICATION_PREFIX,
                timestamp_ms,
                channel_id.as_hyphenated()
            ),
        }
    }
}
