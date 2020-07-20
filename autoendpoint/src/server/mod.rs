//! Main application server

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::metrics;
use crate::server::routers::fcm::router::FcmRouter;
use crate::server::routes::health::{
    health_route, lb_heartbeat_route, status_route, version_route,
};
use crate::server::routes::webpush::webpush_route;
use crate::settings::Settings;
use actix_cors::Cors;
use actix_web::{
    dev, http::StatusCode, middleware::errhandlers::ErrorHandlers, web, App, HttpServer,
};
use autopush_common::db::DynamoStorage;
use cadence::StatsdClient;
use fernet::MultiFernet;
use std::sync::Arc;

mod extractors;
mod headers;
mod routers;
mod routes;

pub use headers::vapid::VapidError;
pub use routers::fcm::settings::FcmSettings;
pub use routers::RouterError;

#[derive(Clone)]
pub struct ServerState {
    /// Server Data
    pub metrics: StatsdClient,
    pub settings: Settings,
    pub fernet: Arc<MultiFernet>,
    pub ddb: DynamoStorage,
    pub http: reqwest::Client,
    pub fcm_router: Arc<FcmRouter>,
}

pub struct Server;

impl Server {
    pub async fn with_settings(settings: Settings) -> ApiResult<dev::Server> {
        let metrics = metrics::metrics_from_opts(&settings)?;
        let bind_address = format!("{}:{}", settings.host, settings.port);
        let fernet = Arc::new(settings.make_fernet());
        let ddb = DynamoStorage::from_opts(
            &settings.message_table_name,
            &settings.router_table_name,
            metrics.clone(),
        )
        .map_err(ApiErrorKind::Database)?;
        let http = reqwest::Client::new();
        let fcm_router = Arc::new(
            FcmRouter::new(
                settings.fcm.clone(),
                settings.endpoint_url.clone(),
                http.clone(),
                metrics.clone(),
            )
            .await?,
        );
        let state = ServerState {
            metrics,
            settings,
            fernet,
            ddb,
            http,
            fcm_router,
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
        .bind(bind_address)?
        .run();

        Ok(server)
    }
}
