use std::str::FromStr;

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::message_id::MessageId;
use crate::extractors::notification::Notification;
use crate::extractors::routers::{RouterType, Routers};
use crate::server::ServerState;
use actix_web::web::Data;
use actix_web::HttpResponse;

/// Handle the `POST /wpush/{api_version}/{token}` and `POST /wpush/{token}` routes
pub async fn webpush_route(
    notification: Notification,
    routers: Routers,
    _state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    let router = routers.get(
        RouterType::from_str(&notification.subscription.user.router_type)
            .map_err(|_| ApiErrorKind::InvalidRouterType)?,
    );

    Ok(router.route_notification(&notification).await?.into())
}

/// Handle the `DELETE /m/{message_id}` route
pub async fn delete_notification_route(
    message_id: MessageId,
    state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    let sort_key = message_id.sort_key();
    debug!("Deleting notification with sort-key {}", sort_key);
    trace!("message_id = {:?}", message_id);
    state
        .ddb
        .remove_message(message_id.uaid(), sort_key)
        .await?;

    Ok(HttpResponse::NoContent().finish())
}
