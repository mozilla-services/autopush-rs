/// Handles most of the web interface aspects including
/// handling dockerflow endpoints, `/push/' and `/notif' endpoints
/// (used for inter-node routing)
/// Also used for MegaphoneUpdater and BroadcastChangeTracker endpoints
#[macro_use]
extern crate slog_scope;

pub mod dockerflow;
pub mod metrics;
pub mod routes;
#[cfg(test)]
mod test;

use actix_web::web;

/// Requires import of the `config` function also in this module to use.
#[macro_export]
macro_rules! build_app {
    ($app_state: expr) => {
        actix_web::App::new()
            .app_data(actix_web::web::Data::new($app_state.clone()))
            .wrap(actix_web::middleware::ErrorHandlers::new().handler(
                actix_http::StatusCode::NOT_FOUND,
                autopush_common::errors::render_404,
            ))
            /*
            .wrap(crate::middleware::sentry::SentryWrapper::new(
                $app_state.metrics.clone(),
                "error".to_owned(),
            ))
                */
            .configure(config)
    };
}

pub fn config(cfg: &mut web::ServiceConfig) {
    cfg
        // Websocket Handler
        .route("/", web::get().to(autoconnect_ws::ws_handler))
        .service(web::resource("/push/{uaid}").route(web::put().to(routes::push_route)))
        .service(web::resource("/notif/{uaid}").route(web::put().to(routes::check_storage_route)))
        .service(web::resource("/status").route(web::get().to(dockerflow::status_route)))
        .service(web::resource("/health").route(web::get().to(dockerflow::health_route)))
        .service(web::resource("/v1/err/crit").route(web::get().to(dockerflow::log_check)))
        // standardized
        .service(web::resource("/__error__").route(web::get().to(dockerflow::log_check)))
        // Dockerflow
        .service(web::resource("/__heartbeat__").route(web::get().to(dockerflow::health_route)))
        .service(
            web::resource("/__lbheartbeat__").route(web::get().to(dockerflow::lb_heartbeat_route)),
        )
        .service(web::resource("/__version__").route(web::get().to(dockerflow::version_route)));
}
