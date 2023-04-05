#[macro_use]
extern crate slog_scope;

use actix_web::{
    http::header::{HeaderValue, USER_AGENT},
    web, Error, HttpRequest, HttpResponse,
};

use autoconnect_settings::AppState;

mod error;
mod handler;
mod session;

/// Handles connected WebSocket clients to a WebPush server
pub async fn ws_handler(
    req: HttpRequest,
    body: web::Payload,
    app_state: web::Data<AppState>,
) -> Result<HttpResponse, Error> {
    let (response, session, msg_stream) = actix_ws::handle(&req, body)?;
    let user_agent = req
        .headers()
        .get(USER_AGENT)
        .unwrap_or(&HeaderValue::from_static(""))
        .to_str()
        .unwrap_or_default()
        .to_owned();
    actix_rt::spawn(handler::webpush_ws(
        session,
        msg_stream,
        app_state.into_inner(),
        user_agent,
    ));
    Ok(response)
}
