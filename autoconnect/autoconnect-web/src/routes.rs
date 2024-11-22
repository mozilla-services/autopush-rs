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
pub async fn push_route(
    uaid: web::Path<Uuid>,
    notif: web::Json<Notification>,
    app_state: web::Data<AppState>,
) -> HttpResponse {
    trace!(
        "⏩ in push_route, uaid: {} channel_id: {}",
        uaid,
        notif.channel_id,
    );
    #[cfg(feature = "reliable_report")]
    let (mut notif, expiry) = {
        let mut notif = notif.into_inner();
        let expiry = Some(notif.timestamp + notif.ttl);
        notif.reliable_state = app_state
            .reliability
            .record(
                &notif.reliability_id,
                autopush_common::reliability::ReliabilityState::IntAccepted,
                &notif.reliable_state,
                expiry,
            )
            .await;
        // Set "transmitted" a bit early since we can't do this inside of `notify`.
        notif.reliable_state = app_state
            .reliability
            .record(
                &notif.reliability_id,
                autopush_common::reliability::ReliabilityState::Transmitted,
                &notif.reliable_state,
                expiry,
            )
            .await;
        (notif, expiry)
    };
    // Attempt to send the notification to the UA using WebSocket protocol, or store on failure.
    let result = app_state
        .clients
        .notify(uaid.into_inner(), notif.clone())
        .await;
    if result.is_ok() {
        #[cfg(feature = "reliable_report")]
        {
            // Set "transmitted" a bit early since we can't do this inside of `notify`.
            notif.reliable_state = app_state
                .reliability
                .record(
                    &notif.reliability_id,
                    autopush_common::reliability::ReliabilityState::Accepted,
                    &notif.reliable_state,
                    expiry,
                )
                .await;
        }

        HttpResponse::Ok().finish()
    } else {
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
