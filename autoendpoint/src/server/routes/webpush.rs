use crate::server::extractors::notification::Notification;
use crate::server::extractors::subscription::Subscription;
use actix_web::HttpResponse;

/// Handle the `/wpush/{api_version}/{token}` and `/wpush/{token}` routes
pub async fn webpush_route(notification: Notification, subscription: Subscription) -> HttpResponse {
    HttpResponse::Ok().finish()
}
