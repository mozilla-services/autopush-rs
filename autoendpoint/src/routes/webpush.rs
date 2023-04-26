use std::str::FromStr;

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::message_id::MessageId;
use crate::extractors::notification::Notification;
use crate::extractors::routers::{RouterType, Routers};
use crate::server::AppState;
use actix_web::web::Data;
use actix_web::HttpResponse;
use cadence::CountedExt;

/// Handle the `POST /wpush/{api_version}/{token}` and `POST /wpush/{token}` routes
pub async fn webpush_route(
    notification: Notification,
    routers: Routers,
    app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    // TODO:
    sentry::configure_scope(|scope| {
        scope.set_extra(
            "uaid",
            notification.subscription.user.uaid.to_string().into(),
        );
    });
    let router = routers.get(
        RouterType::from_str(&notification.subscription.user.router_type)
            .map_err(|_| ApiErrorKind::InvalidRouterType)?,
    );
    if notification.has_topic() {
        app_state.metrics.incr("ua.notification.topic")?;
    }
    Ok(router.route_notification(&notification).await?.into())
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
