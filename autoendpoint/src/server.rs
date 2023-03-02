//! Main application server
use std::sync::Arc;
use std::time::Duration;

use autopush_common::db::dynamodb::DdbClientImpl;
use autopush_common::db::DbSettings;
use autopush_common::db::{client::DbClient, StorageType};

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::metrics;
// TODO: sentry is currently broken. Need to investgate solution
// use crate::middleware::sentry::sentry_middleware;
use crate::routers::adm::router::AdmRouter;
use crate::routers::apns::router::ApnsRouter;
use crate::routers::fcm::router::FcmRouter;
use crate::routes::health::{
    health_route, lb_heartbeat_route, log_check, status_route, version_route,
};
use crate::routes::registration::{
    get_channels_route, new_channel_route, register_uaid_route, unregister_channel_route,
    unregister_user_route, update_token_route,
};
use crate::routes::webpush::{delete_notification_route, webpush_route};
use crate::settings::Settings;

use actix_cors::Cors;
use actix_web::{dev, http::StatusCode, middleware::ErrorHandlers, web, App, HttpServer};
use cadence::StatsdClient;
use fernet::MultiFernet;

#[derive(Clone)]
pub struct ServerState {
    /// Server Data
    pub metrics: Arc<StatsdClient>,
    pub settings: Settings,
    pub fernet: MultiFernet,
    // TODO: Convert this to the autopush_common dbCommandClient impl.
    pub dbclient: Box<dyn DbClient>,
    pub http: reqwest::Client,
    pub fcm_router: Arc<FcmRouter>,
    pub apns_router: Arc<ApnsRouter>,
    pub adm_router: Arc<AdmRouter>,
}

pub struct Server;

impl Server {
    pub async fn with_settings(settings: Settings) -> ApiResult<dev::Server> {
        let metrics = Arc::new(metrics::metrics_from_opts(&settings)?);
        let bind_address = format!("{}:{}", settings.host, settings.port);
        let fernet = settings.make_fernet();
        let endpoint_url = settings.endpoint_url();
        let db_settings = DbSettings {
            dsn: settings.db_dsn.clone(),
            db_settings: settings.db_settings.clone(),
        };
        let ddb: Box<dyn DbClient> = match StorageType::from_dsn(&db_settings.dsn) {
            StorageType::DynamoDb => Box::new(
                DdbClientImpl::new(metrics.clone(), &db_settings)
                    .map_err(|e| ApiErrorKind::Database(e.into()))?,
            ),
            StorageType::INVALID => {
                return Err(ApiErrorKind::General("Invalid DSN specified".to_owned()).into())
            }
        };
        let http = reqwest::ClientBuilder::new()
            .connect_timeout(Duration::from_millis(settings.connection_timeout_millis))
            .timeout(Duration::from_millis(settings.request_timeout_millis))
            .build()
            .expect("Could not generate request client");
        let fcm_router = Arc::new(
            FcmRouter::new(
                settings.fcm.clone(),
                endpoint_url.clone(),
                http.clone(),
                metrics.clone(),
                ddb.clone(),
            )
            .await?,
        );
        let apns_router = Arc::new(
            ApnsRouter::new(
                settings.apns.clone(),
                endpoint_url.clone(),
                metrics.clone(),
                ddb.clone(),
            )
            .await?,
        );
        let adm_router = Arc::new(AdmRouter::new(
            settings.adm.clone(),
            endpoint_url,
            http.clone(),
            metrics.clone(),
            ddb.clone(),
        )?);
        let state = ServerState {
            metrics,
            settings,
            fernet,
            dbclient: ddb,
            http,
            fcm_router,
            apns_router,
            adm_router,
        };

        let server = HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(state.clone()))
                // Middleware
                .wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, ApiError::render_404))
                // .wrap_fn(sentry_middleware)
                .wrap(Cors::default())
                // Extractor configuration
                //  TODO: web::Bytes::configure was removed. What did this do? Can we just pull from state?
                // .app_data(web::Bytes::configure(|cfg| {
                //    cfg.limit(state.settings.max_data_bytes)
                // }))
                .app_data(web::JsonConfig::default().limit(state.settings.max_data_bytes))
                // Endpoints
                .service(
                    web::resource(["/wpush/{api_version}/{token}", "/wpush/{token}"])
                        .route(web::post().to(webpush_route)),
                )
                .service(
                    web::resource("/m/{message_id}")
                        .route(web::delete().to(delete_notification_route)),
                )
                .service(
                    web::resource("/v1/{router_type}/{app_id}/registration")
                        .route(web::post().to(register_uaid_route)),
                )
                .service(
                    web::resource("/v1/{router_type}/{app_id}/registration/{uaid}")
                        .route(web::put().to(update_token_route))
                        .route(web::get().to(get_channels_route))
                        .route(web::delete().to(unregister_user_route)),
                )
                .service(
                    web::resource("/v1/{router_type}/{app_id}/registration/{uaid}/subscription")
                        .route(web::post().to(new_channel_route)),
                )
                .service(
                    web::resource(
                        "/v1/{router_type}/{app_id}/registration/{uaid}/subscription/{chid}",
                    )
                    .route(web::delete().to(unregister_channel_route)),
                )
                // Health checks
                .service(web::resource("/status").route(web::get().to(status_route)))
                .service(web::resource("/health").route(web::get().to(health_route)))
                // legacy
                .service(web::resource("/v1/err").route(web::get().to(log_check)))
                // standardized
                .service(web::resource("/__error__").route(web::get().to(log_check)))
                // Dockerflow
                .service(web::resource("/__heartbeat__").route(web::get().to(health_route)))
                .service(web::resource("/__lbheartbeat__").route(web::get().to(lb_heartbeat_route)))
                .service(web::resource("/__version__").route(web::get().to(version_route)))
        })
        .bind(bind_address)?
        .run();

        Ok(server)
    }
}
