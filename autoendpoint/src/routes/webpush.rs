use crate::error::ApiResult;
use crate::extractors::message_id::MessageId;
use crate::extractors::notification::Notification;
use crate::extractors::routers::Routers;
use actix_web::HttpResponse;

/// Handle the `POST /wpush/{api_version}/{token}` and `POST /wpush/{token}` routes
pub async fn webpush_route(
    notification: Notification,
    routers: Routers,
) -> ApiResult<HttpResponse> {
    let router = routers.get(notification.subscription.router_type);

    let response = router.route_notification(&notification).await?;

    Ok(response.into())
}

/// Handle the `DELETE /m/{message_id}` route
pub async fn delete_notification_route(message_id: MessageId) -> ApiResult<HttpResponse> {
    Ok(HttpResponse::Ok().finish())
}
