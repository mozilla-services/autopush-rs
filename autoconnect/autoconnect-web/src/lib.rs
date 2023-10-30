/// Handles most of the web interface aspects including
/// handling dockerflow endpoints, `/push/' and `/notif' endpoints
/// (used for inter-node routing)
/// Also used for MegaphoneUpdater and BroadcastChangeTracker endpoints
#[macro_use]
extern crate slog_scope;

pub mod dockerflow;
pub mod error;
pub mod metrics;
pub mod routes;
#[cfg(test)]
mod test;

use actix_web::web;

#[macro_export]
macro_rules! build_app {
    ($app_state: expr, $config: expr) => {
        actix_web::App::new()
            .app_data(actix_web::web::Data::new($app_state.clone()))
            .wrap(actix_web::middleware::ErrorHandlers::new().handler(
                actix_http::StatusCode::NOT_FOUND,
                autopush_common::errors::render_404,
            ))
            .wrap(autopush_common::middleware::sentry::SentryWrapper::<
                $crate::error::ApiError,
            >::new(
                $app_state.metrics.clone(), "error".to_owned()
            ))
            .configure($config)
    };
}

/// The publicly exposed app config
pub fn config(cfg: &mut web::ServiceConfig) {
    cfg
        // Websocket Handler
        .route("/", web::get().to(routes::ws_route))
        .service(web::scope("").configure(dockerflow::service));
}

/// The internal router app config
pub fn config_router(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/push/{uaid}").route(web::put().to(routes::push_route)))
        .service(web::resource("/notif/{uaid}").route(web::put().to(routes::check_storage_route)))
        .service(web::scope("").configure(dockerflow::service));
}
