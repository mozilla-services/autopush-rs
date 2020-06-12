use crate::server::extractors::notification::Notification;
use crate::server::extractors::subscription::Subscription;
use actix_web::HttpResponse;

/// Handle the `/wpush/{api_version}/{token}` and `/wpush/{token}` routes
pub async fn webpush_route(
    _notification: Notification,
    _subscription: Subscription,
) -> HttpResponse {
    HttpResponse::Ok().finish()
}
