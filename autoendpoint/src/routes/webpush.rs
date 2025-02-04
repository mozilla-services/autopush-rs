use std::str::FromStr;

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::message_id::MessageId;
use crate::extractors::notification::Notification;
use crate::extractors::routers::{RouterType, Routers};
use crate::server::AppState;

use actix_web::web::Data;
use actix_web::HttpResponse;
use opentelemetry::{
    global,
    trace::{Span, SpanKind, Tracer},
};

/// Handle the `POST /wpush/{api_version}/{token}` and `POST /wpush/{token}` routes
/// This is the endpoint for all incoming Push subscription updates.
pub async fn webpush_route(
    notification: Notification,
    routers: Routers,
    _app_state: Data<AppState>,
) -> ApiResult<HttpResponse> {
    sentry::configure_scope(|scope| {
        scope.set_extra(
            "uaid",
            notification.subscription.user.uaid.to_string().into(),
        );
    });

    let tracer = global::tracer("autoendpoint");
    // XXX: /wpush/{api_version}?
    let mut span = tracer
        .span_builder("wpush")
        .with_kind(SpanKind::Server)
        .start(&tracer);
    span.add_event("POST notification", vec![]);

    let router = routers.get(
        RouterType::from_str(&notification.subscription.user.router_type)
            .map_err(|_| ApiErrorKind::InvalidRouterType)?,
    );
    Ok(router.route_notification(notification).await?.into())
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
