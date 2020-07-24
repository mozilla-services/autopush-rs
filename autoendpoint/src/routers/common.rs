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
