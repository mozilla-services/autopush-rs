use actix_web::{web, HttpRequest, HttpResponse};
use uuid::Uuid;

use autoconnect_settings::AppState;
use autopush_common::notification::Notification;

use crate::error::ApiError;

/// Handle WebSocket WebPush clients
pub async fn ws_route(
    req: HttpRequest,
    body: web::Payload,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, ApiError> {
    Ok(autoconnect_ws::ws_handler(req, body, app_state).await?)
}

/// Deliver a Push notification directly to a connected client
#[allow(unused_mut)]
pub async fn push_route(
    uaid: web::Path<Uuid>,
    mut notif: web::Json<Notification>,
    app_state: web::Data<AppState>,
) -> HttpResponse {
    trace!(
        "⏩ in push_route, uaid: {} channel_id: {}",
        uaid,
        notif.channel_id,
    );
    #[cfg(feature = "reliable_report")]
    {
        notif
            .record_reliability(
                &app_state.reliability,
                autopush_common::reliability::ReliabilityState::IntAccepted,
            )
            .await;
        notif
            .record_reliability(
                &app_state.reliability,
                autopush_common::reliability::ReliabilityState::Transmitted,
            )
            .await;
    }
    // Attempt to send the notification to the UA using WebSocket protocol, or store on failure.
    // NOTE: Since this clones the notification, there is a potential to
    // double count the reliability state.
    #[cfg(feature = "reliable_report")]
    let notif_clone  = notif.clone_without_reliability_state();
    #[cfg(not(feature = "reliable_report"))]
    let notif_clone = notif.clone();
    let result = app_state
        .clients
        .notify(uaid.into_inner(), notif_clone)
        .await;
    if result.is_ok() {
        #[cfg(feature = "reliable_report")]
        notif
            .record_reliability(
                &app_state.reliability,
                autopush_common::reliability::ReliabilityState::Accepted,
            )
            .await;
        HttpResponse::Ok().finish()
    } else {
        #[cfg(feature = "reliable_report")]
        notif
            .record_reliability(
                &app_state.reliability,
                autopush_common::reliability::ReliabilityState::Errored,
            )
            .await;
        HttpResponse::NotFound().body("Client not available")
    }
}

/// Notify a connected client to check storage for new notifications
pub async fn check_storage_route(
    uaid: web::Path<Uuid>,
    app_state: web::Data<AppState>,
) -> HttpResponse {
    trace!("⏩ check_storage_route, uaid: {}", uaid);
    let result = app_state.clients.check_storage(uaid.into_inner()).await;
    if result.is_ok() {
        HttpResponse::Ok().finish()
    } else {
        HttpResponse::NotFound().body("Client not available")
    }
}
