use actix_web::HttpRequest;
use fernet::MultiFernet;
use uuid::Uuid;

use crate::errors::{ApcErrorKind, Result};
use crate::notification::{STANDARD_NOTIFICATION_PREFIX, TOPIC_NOTIFICATION_PREFIX};

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
        timestamp: u64,
    },
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
                timestamp,
            } => format!(
                "{}:{}:{}:{}",
                STANDARD_NOTIFICATION_PREFIX,
                uaid.as_simple(),
                channel_id.as_simple(),
                timestamp
            ),
        };

        fernet.encrypt(id_str.as_bytes())
    }

    /// Decrypt and decode the message ID
    pub fn decrypt(fernet: &MultiFernet, message_id: &str) -> Result<Self> {
        let decrypted_bytes = fernet
            .decrypt(message_id)
            .map_err(|_| ApcErrorKind::InvalidMessageId)?;
        let decrypted_str = String::from_utf8_lossy(&decrypted_bytes);
        let segments: Vec<_> = decrypted_str.split(':').collect();

        if segments.len() != 4 {
            return Err(ApcErrorKind::InvalidMessageId.into());
        }

        let (version, uaid, chid, topic_or_timestamp) =
            (segments[0], segments[1], segments[2], segments[3]);

        match version {
            "01" => Ok(MessageId::WithTopic {
                uaid: Uuid::parse_str(uaid).map_err(|_| ApcErrorKind::InvalidMessageId)?,
                channel_id: Uuid::parse_str(chid).map_err(|_| ApcErrorKind::InvalidMessageId)?,
                topic: topic_or_timestamp.to_string(),
            }),
            "02" => Ok(MessageId::WithoutTopic {
                uaid: Uuid::parse_str(uaid).map_err(|_| ApcErrorKind::InvalidMessageId)?,
                channel_id: Uuid::parse_str(chid).map_err(|_| ApcErrorKind::InvalidMessageId)?,
                timestamp: topic_or_timestamp
                    .parse()
                    .map_err(|_| ApcErrorKind::InvalidMessageId)?,
            }),
            _ => Err(ApcErrorKind::InvalidMessageId.into()),
        }
    }

    pub fn from_request(fernet: &MultiFernet, req: HttpRequest) -> Result<Self> {
        let message_id = req
            .match_info()
            .get("message_id")
            .expect("{message_id} must be part of the path");

        Self::decrypt(fernet, message_id)
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
                timestamp,
                ..
            } => format!(
                "{}:{}:{}",
                STANDARD_NOTIFICATION_PREFIX,
                timestamp,
                channel_id.as_hyphenated()
            ),
        }
    }
}
