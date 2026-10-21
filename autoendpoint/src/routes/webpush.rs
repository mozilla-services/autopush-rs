use std::str::FromStr;

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::extractors::message_id::MessageId;
use crate::extractors::notification::Notification;
use crate::extractors::routers::{RouterType, Routers};
use crate::routers::RouterError;
use crate::server::AppState;
use actix_http::StatusCode;
use actix_web::HttpResponse;
use actix_web::web::Data;
use autopush_common::delivery_metrics::{DeliveryPath, DeliveryRoute, Outcome, emit_outcome};
use autopush_common::util::ms_since_epoch;

/// Handle the `POST /wpush/{api_version}/{token}` and `POST /wpush/{token}` routes
/// This is the endpoint for all incoming Push subscription updates.
pub async fn webpush_route(
    notification: Result<Notification, ApiError>,
    routers: Routers,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    let notification = notification.inspect_err(|e| {
        record_error_outcome(&app_state, DeliveryPath::Unknown, e);
    })?;
    sentry::configure_scope(|scope| {
        scope.set_extra(
            "uaid",
            notification.subscription.user.uaid.to_string().into(),
        );
    });
    let router_type = RouterType::from_str(&notification.subscription.user.router_type)
        .map_err(|_| ApiErrorKind::InvalidRouterType)?;
    let path = delivery_path(&router_type);
    let received_ms = notification.sort_key_timestamp;
    let router = routers.get(router_type);
    let result = router.route_notification(notification).await;

    if let Some(path) = path {
        match &result {
            // WebPush deliveries are completed by autoconnect on the client's
            // ACK. TTL=0 expiry is recorded by the router.
            Ok(_) if path != DeliveryPath::Ws => {
                app_state.delivery_metrics.observe_delivery(
                    path,
                    DeliveryRoute::Direct,
                    received_ms,
                    ms_since_epoch(),
                );
                emit_outcome(&app_state.metrics, path, Outcome::Delivered);
            }
            Ok(_) => {}
            Err(e) => record_error_outcome(&app_state, path, e),
        }
    }
    Ok(result?.into())
}

/// The `path` metric tag for a router. Stub router traffic is excluded.
fn delivery_path(router_type: &RouterType) -> Option<DeliveryPath> {
    match router_type {
        RouterType::WebPush => Some(DeliveryPath::Ws),
        RouterType::FCM | RouterType::GCM => Some(DeliveryPath::Fcm),
        RouterType::APNS => Some(DeliveryPath::Apns),
        #[cfg(feature = "stub")]
        RouterType::STUB => None,
    }
}

fn record_error_outcome(app_state: &AppState, path: DeliveryPath, error: &ApiError) {
    if let Some(outcome) = error_outcome(&error.kind) {
        emit_outcome(&app_state.metrics, path, outcome);
    }
}

/// Classify a failed notification. An undecryptable token and 410 describe
/// the recipient's subscription, and bridge errors are attributed to FCM/APNs.
/// Other 4xx responses (including the 404 for an unknown API version) are
/// defects in the sender's request (URL, headers, VAPID, payload size) and say
/// nothing about push or the recipient, so they're not counted.
fn error_outcome(kind: &ApiErrorKind) -> Option<Outcome> {
    if let ApiErrorKind::Router(RouterError::SaveDb(..)) = kind {
        return Some(Outcome::StorageError);
    }
    if let ApiErrorKind::InvalidToken = kind {
        return Some(Outcome::NotFound);
    }
    let status = kind.status();
    if status == StatusCode::GONE {
        return Some(Outcome::Gone);
    }
    if let ApiErrorKind::Router(
        RouterError::Apns(_)
        | RouterError::Fcm(_)
        | RouterError::Authentication
        | RouterError::RequestTimeout
        | RouterError::Connect(_),
    ) = kind
    {
        return Some(Outcome::BridgeError);
    }
    status.is_server_error().then_some(Outcome::InternalError)
}

/// Handle the `DELETE /m/{message_id}` route
pub async fn delete_notification_route(
    message_id: MessageId,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    let sort_key = message_id.sort_key();
    debug!("Deleting notification with sort-key {}", sort_key);
    trace!("message_id = {:?}", message_id);
    app_state
        .db
        .remove_message(&message_id.uaid(), &sort_key)
        .await?;

    Ok(HttpResponse::NoContent().finish())
}

#[cfg(test)]
mod tests {
    use autopush_common::db::error::DbError;

    use super::*;
    use crate::routers::{apns::error::ApnsError, fcm::error::FcmError};

    #[test]
    fn classify_error_outcomes() {
        let cases = [
            (ApiErrorKind::InvalidToken, Some(Outcome::NotFound)),
            (ApiErrorKind::NoSubscription, Some(Outcome::Gone)),
            (ApiErrorKind::NoUser, Some(Outcome::Gone)),
            (
                ApiErrorKind::Router(RouterError::UserWasDeleted),
                Some(Outcome::Gone),
            ),
            (
                ApiErrorKind::Router(RouterError::NotFound),
                Some(Outcome::Gone),
            ),
            (
                ApiErrorKind::Router(RouterError::Apns(ApnsError::Unregistered)),
                Some(Outcome::Gone),
            ),
            (
                ApiErrorKind::Router(RouterError::Fcm(FcmError::InvalidAppId("x".to_owned()))),
                Some(Outcome::BridgeError),
            ),
            (
                ApiErrorKind::Router(RouterError::RequestTimeout),
                Some(Outcome::BridgeError),
            ),
            (
                ApiErrorKind::Router(RouterError::SaveDb(
                    DbError::General("oops".to_owned()),
                    None,
                )),
                Some(Outcome::StorageError),
            ),
            (
                ApiErrorKind::General("oops".to_owned()),
                Some(Outcome::InternalError),
            ),
            (ApiErrorKind::NoTTL, None),
            (ApiErrorKind::InvalidApiVersion, None),
            (ApiErrorKind::InvalidRouterType, None),
            (ApiErrorKind::Router(RouterError::TooMuchData(10)), None),
        ];
        for (kind, expected) in cases {
            assert_eq!(error_outcome(&kind), expected, "{kind:?}");
        }
    }
}
