//! Main application server

use actix_cors::Cors;
use actix_web::{
    dev, http::StatusCode, middleware::errhandlers::ErrorHandlers, web, App, HttpServer,
};
use cadence::StatsdClient;

use crate::error::{ApiError, ApiResult};
use crate::metrics;
use crate::server::routes::health::{
    health_route, lb_heartbeat_route, status_route, version_route,
};
use crate::server::routes::webpush::webpush_route;
use crate::settings::Settings;
use fernet::MultiFernet;
use std::sync::Arc;

mod extractors;
mod headers;
mod routes;

#[derive(Clone)]
pub struct ServerState {
    /// Server Data
    pub metrics: StatsdClient,
    pub port: u16,
    pub fernet: Arc<MultiFernet>,
}

pub struct Server;

impl Server {
    pub fn with_settings(settings: Settings) -> ApiResult<dev::Server> {
        let metrics = metrics::metrics_from_opts(&settings)?;
        let port = settings.port;
        let state = ServerState {
            metrics,
            port,
            fernet: Arc::new(settings.make_fernet()),
        };

        let server = HttpServer::new(move || {
            App::new()
                .data(state.clone())
                .wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, ApiError::render_404))
                .wrap(Cors::default())
                // Endpoints
                .service(
                    web::resource(["/wpush/{api_version}/{token}", "/wpush/{token}"])
                        .route(web::post().to(webpush_route)),
                )
                // Health checks
                .service(web::resource("/status").route(web::get().to(status_route)))
                .service(web::resource("/health").route(web::get().to(health_route)))
                // Dockerflow
                .service(web::resource("/__heartbeat__").route(web::get().to(status_route)))
                .service(web::resource("/__lbheartbeat__").route(web::get().to(lb_heartbeat_route)))
                .service(web::resource("/__version__").route(web::get().to(version_route)))
        })
        .bind(format!("{}:{}", settings.host, settings.port))?
        .run();

        Ok(server)
    }
}
