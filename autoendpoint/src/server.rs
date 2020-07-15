//! Main application server

use crate::db::client::DbClient;
use crate::error::{ApiError, ApiResult};
use crate::metrics;
use crate::routers::fcm::router::FcmRouter;
use crate::routes::health::{health_route, lb_heartbeat_route, status_route, version_route};
use crate::routes::webpush::{delete_notification_route, webpush_route};
use crate::settings::Settings;
use actix_cors::Cors;
use actix_web::{
    dev, http::StatusCode, middleware::errhandlers::ErrorHandlers, web, App, HttpServer,
};
use cadence::StatsdClient;
use fernet::MultiFernet;
use std::sync::Arc;

#[derive(Clone)]
pub struct ServerState {
    /// Server Data
    pub metrics: StatsdClient,
    pub settings: Settings,
    pub fernet: Arc<MultiFernet>,
    pub ddb: DbClient,
    pub http: reqwest::Client,
    pub fcm_router: Arc<FcmRouter>,
}

pub struct Server;

impl Server {
    pub async fn with_settings(settings: Settings) -> ApiResult<dev::Server> {
        let metrics = metrics::metrics_from_opts(&settings)?;
        let bind_address = format!("{}:{}", settings.host, settings.port);
        let fernet = Arc::new(settings.make_fernet());
        let ddb = DbClient::new(
            metrics.clone(),
            settings.router_table_name.clone(),
            settings.message_table_name.clone(),
        )?;
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
                .service(
                    web::resource("/m/{message_id}")
                        .route(web::delete().to(delete_notification_route)),
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
