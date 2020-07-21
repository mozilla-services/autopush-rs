use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::routers::Routers;
use actix_web::HttpResponse;

/// Handle the `/wpush/{api_version}/{token}` and `/wpush/{token}` routes
pub async fn webpush_route(
    notification: Notification,
    routers: Routers,
) -> ApiResult<HttpResponse> {
    let router = routers.get(notification.subscription.router_type);

    let response = router.route_notification(&notification).await?;

    Ok(response.into())
}
