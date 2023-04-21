/// Handle incoming notifications from autoendpoint.
use actix_web::{web, HttpResponse};
use uuid::Uuid;

use autoconnect_settings::AppState;
use autopush_common::notification::Notification;

/// Deliver a Push notification directly to a connected client
pub async fn push_route(
    uaid: web::Path<Uuid>,
    notif: web::Json<Notification>,
    state: web::Data<AppState>,
) -> HttpResponse {
    trace!(
        "push_route, uaid: {} channel_id: {}",
        uaid,
        notif.channel_id
    );
    let result = state
        .clients
        .notify(uaid.into_inner(), notif.into_inner())
        .await
        /*.map_or_else(
            || HttpResponse::NotFound().body("Client not available"),
            |_| HttpResponse::Ok().finish()
        )*/
    if result.is_ok() {
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::NotFound().body("Client not available")
    }
}

/// Notify a connected client to check storage for new notifications
pub async fn check_storage_route(
    uaid: web::Path<Uuid>,
    state: web::Data<AppState>,
) -> HttpResponse {
    trace!("check_storage_route, uaid: {}", uaid);
    let result = state.clients.check_storage(uaid.into_inner()).await;
    if result.is_ok() {
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::NotFound().body("Client not available")
    }
}
