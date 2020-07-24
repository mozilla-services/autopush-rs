use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::routers::RouterError;
use autopush_common::util::InsertOpt;
use std::collections::HashMap;

/// Convert a notification into a WebPush message
pub fn build_message_data(
    notification: &Notification,
    max_data: usize,
) -> ApiResult<HashMap<&'static str, String>> {
    let mut message_data = HashMap::new();
    message_data.insert("chid", notification.subscription.channel_id.to_string());

    // Only add the other headers if there's data
    if let Some(data) = &notification.data {
        if data.len() > max_data {
            // Too much data. Tell the client how many bytes extra they had.
            return Err(RouterError::TooMuchData(data.len() - max_data).into());
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

/// Common router test code
#[cfg(test)]
pub mod tests {
    use crate::extractors::notification::Notification;
    use crate::extractors::notification_headers::NotificationHeaders;
    use crate::extractors::routers::RouterType;
    use crate::extractors::subscription::Subscription;
    use autopush_common::db::DynamoDbUser;
    use std::collections::HashMap;
    use uuid::Uuid;

    pub const CHANNEL_ID: &str = "deadbeef-13f9-4639-87f9-2ff731824f34";

    /// Get the test channel ID as a Uuid
    pub fn channel_id() -> Uuid {
        Uuid::parse_str(CHANNEL_ID).unwrap()
    }

    /// Create a notification
    pub fn make_notification(
        router_data: HashMap<String, serde_json::Value>,
        data: Option<String>,
        router_type: RouterType,
    ) -> Notification {
        Notification {
            message_id: "test-message-id".to_string(),
            subscription: Subscription {
                user: DynamoDbUser {
                    router_data: Some(router_data),
                    ..Default::default()
                },
                channel_id: channel_id(),
                router_type,
                vapid: None,
            },
            headers: NotificationHeaders {
                ttl: 0,
                topic: Some("test-topic".to_string()),
                encoding: Some("test-encoding".to_string()),
                encryption: Some("test-encryption".to_string()),
                encryption_key: Some("test-encryption-key".to_string()),
                crypto_key: Some("test-crypto-key".to_string()),
            },
            timestamp: 0,
            data,
        }
    }
}
