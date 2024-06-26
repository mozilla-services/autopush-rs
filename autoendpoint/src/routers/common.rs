use crate::error::{ApiError, ApiResult};
use crate::extractors::notification::Notification;
use crate::headers::vapid::VapidHeaderWithKey;
use crate::routers::RouterError;
use actix_web::http::StatusCode;
use autopush_common::db::client::DbClient;
use autopush_common::errors::ReportableError;
use autopush_common::util::InsertOpt;
use cadence::{Counted, CountedExt, StatsdClient, Timed};
use std::collections::HashMap;
use uuid::Uuid;

/// Convert a notification into a WebPush message
pub fn build_message_data(notification: &Notification) -> ApiResult<HashMap<&'static str, String>> {
    let mut message_data = HashMap::new();
    message_data.insert("chid", notification.subscription.channel_id.to_string());

    // Only add the other headers if there's data
    if let Some(data) = &notification.data {
        message_data.insert("body", data.clone());
        message_data.insert_opt("con", notification.headers.encoding.as_ref());
        message_data.insert_opt("enc", notification.headers.encryption.as_ref());
        message_data.insert_opt("cryptokey", notification.headers.crypto_key.as_ref());
        message_data.insert_opt("enckey", notification.headers.encryption_key.as_ref());
    }

    Ok(message_data)
}

/// Check the data against the max data size and return an error if there is too
/// much data.
pub fn message_size_check(data: &[u8], max_data: usize) -> Result<(), RouterError> {
    if data.len() > max_data {
        trace!("Data is too long by {} bytes", data.len() - max_data);
        Err(RouterError::TooMuchData(data.len() - max_data))
    } else {
        Ok(())
    }
}

/// Handle a bridge error by logging, updating metrics, etc
pub async fn handle_error(
    error: RouterError,
    metrics: &StatsdClient,
    db: &dyn DbClient,
    platform: &str,
    app_id: &str,
    uaid: Uuid,
    vapid: Option<VapidHeaderWithKey>,
) -> ApiError {
    match &error {
        RouterError::Authentication => {
            error!("Bridge authentication error");
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "authentication",
                error.status(),
                error.errno(),
            );
        }
        RouterError::GCMAuthentication => {
            warn!("GCM Authentication error");
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "gcm authentication",
                error.status(),
                error.errno(),
            );
        }
        RouterError::RequestTimeout => {
            // Bridge timeouts are common.
            info!("Bridge timeout");
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "timeout",
                error.status(),
                error.errno(),
            );
        }
        RouterError::Connect(e) => {
            warn!("Bridge unavailable: {}", e);
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "connection_unavailable",
                error.status(),
                error.errno(),
            );
        }
        RouterError::NotFound => {
            debug!("Bridge recipient not found, removing user");
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "recipient_gone",
                error.status(),
                error.errno(),
            );

            if let Err(e) = db.remove_user(&uaid).await {
                warn!("Error while removing user due to bridge not_found: {}", e);
            }
        }
        RouterError::Upstream { .. } => {
            warn!("{}", error.to_string());
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "server_error",
                error.status(),
                error.errno(),
            );
        }
        RouterError::TooMuchData(_) => {
            // Do not log this error since it's fairly common.
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "too_much_data",
                error.status(),
                error.errno(),
            );
        }
        _ => {
            warn!("Unknown error while sending bridge request: {}", error);
            incr_error_metric(
                metrics,
                platform,
                app_id,
                "unknown",
                error.status(),
                error.errno(),
            );
        }
    }

    let mut err = ApiError::from(error);

    if let Some(Ok(claims)) = vapid.map(|v| v.vapid.claims()) {
        let mut extras = err.extras.unwrap_or_default();
        extras.extend([("sub".to_owned(), claims.sub)]);
        err.extras = Some(extras);
    };
    err
}

/// Increment `notification.bridge.error`
pub fn incr_error_metric(
    metrics: &StatsdClient,
    platform: &str,
    app_id: &str,
    reason: &str,
    status: StatusCode,
    errno: Option<usize>,
) {
    // I'd love to extract the status and errno from the passed ApiError, but a2 error handling makes that impossible.
    metrics
        .incr_with_tags("notification.bridge.error")
        .with_tag("platform", platform)
        .with_tag("app_id", app_id)
        .with_tag("reason", reason)
        .with_tag("error", &status.to_string())
        .with_tag("errno", &errno.unwrap_or(0).to_string())
        .send();
}

/// Update metrics after successfully routing the notification
pub fn incr_success_metrics(
    metrics: &StatsdClient,
    platform: &str,
    app_id: &str,
    notification: &Notification,
) {
    metrics
        .incr_with_tags("notification.bridge.sent")
        .with_tag("platform", platform)
        .with_tag("app_id", app_id)
        .send();
    metrics
        .count_with_tags(
            "notification.message_data",
            notification.data.as_ref().map(String::len).unwrap_or(0) as i64,
        )
        .with_tag("platform", platform)
        .with_tag("app_id", app_id)
        .with_tag("destination", "Direct")
        .send();
    metrics
        .time_with_tags(
            "notification.total_request_time",
            (autopush_common::util::sec_since_epoch() - notification.timestamp) * 1000,
        )
        .with_tag("platform", platform)
        .with_tag("app_id", app_id)
        .send();
}

/// Common router test code
#[cfg(test)]
pub mod tests {
    use crate::extractors::notification::Notification;
    use crate::extractors::notification_headers::NotificationHeaders;
    use crate::extractors::routers::RouterType;
    use crate::extractors::subscription::Subscription;
    use autopush_common::db::User;
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
                user: User {
                    router_data: Some(router_data),
                    router_type: router_type.to_string(),
                    ..Default::default()
                },
                channel_id: channel_id(),
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
            sort_key_timestamp: 0,
            data,
        }
    }
}
