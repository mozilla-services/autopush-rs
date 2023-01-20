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
    mut notification: Notification,
    routers: Routers,
    state: Data<ServerState>,
) -> ApiResult<HttpResponse> {
    let router = routers.get(
        RouterType::from_str(&notification.subscription.user.router_type)
            .map_err(|_| ApiErrorKind::InvalidRouterType)?,
    );

    let resp = router.route_notification(&notification).await?;
    if router.is_mobile() {
        // update the `expiry` timestamp for the user.
        // Here, we'll use a successful bridged notification route as a proxy
        // for the UA having connected to us. `expiry` allows us to automatically
        // age  out "orphaned" push endpoints.
        state
            .ddb
            .update_user(&mut notification.subscription.user)
            .await?;
    };
    Ok(resp.into())
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
