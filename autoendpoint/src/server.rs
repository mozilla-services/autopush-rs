//! Main application server
#![forbid(unsafe_code)]
use std::sync::Arc;
use std::time::Duration;

use actix_cors::Cors;
use actix_web::{
    dev, http::StatusCode, middleware::ErrorHandlers, web, web::Data, App, HttpServer,
};
use cadence::StatsdClient;
use fernet::MultiFernet;
use serde_json::json;

#[cfg(feature = "bigtable")]
use autopush_common::db::bigtable::BigTableClientImpl;
#[cfg(feature = "dual")]
use autopush_common::db::dual::DualClientImpl;
#[cfg(feature = "dynamodb")]
use autopush_common::db::dynamodb::DdbClientImpl;
use autopush_common::{
    db::{client::DbClient, spawn_pool_periodic_reporter, DbSettings, StorageType},
    middleware::sentry::SentryWrapper,
};

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::metrics;
#[cfg(feature = "adm")]
use crate::routers::adm::router::AdmRouter;
use crate::routers::{apns::router::ApnsRouter, fcm::router::FcmRouter};
use crate::routes::{
    health::{health_route, lb_heartbeat_route, log_check, status_route, version_route},
    registration::{
        get_channels_route, new_channel_route, register_uaid_route, unregister_channel_route,
        unregister_user_route, update_token_route,
    },
    webpush::{delete_notification_route, webpush_route},
};
use crate::settings::Settings;

#[derive(Clone)]
pub struct AppState {
    /// Server Data
    pub metrics: Arc<StatsdClient>,
    pub settings: Settings,
    pub fernet: MultiFernet,
    pub db: Box<dyn DbClient>,
    pub http: reqwest::Client,
    pub fcm_router: Arc<FcmRouter>,
    pub apns_router: Arc<ApnsRouter>,
    #[cfg(feature = "adm")]
    pub adm_router: Arc<AdmRouter>,
}

pub struct Server;

impl Server {
    pub async fn with_settings(settings: Settings) -> ApiResult<dev::Server> {
        let metrics = Arc::new(metrics::metrics_from_settings(&settings)?);
        let bind_address = format!("{}:{}", settings.host, settings.port);
        let fernet = settings.make_fernet();
        let endpoint_url = settings.endpoint_url();
        let db_settings = DbSettings {
            dsn: settings.db_dsn.clone(),
            db_settings: if settings.db_settings.is_empty() {
                warn!("❗ Using obsolete message_table and router_table args");
                // backfill from the older arguments.
                json!({"message_table": settings.message_table_name, "router_table":settings.router_table_name}).to_string()
            } else {
                settings.db_settings.clone()
            },
        };
        let db: Box<dyn DbClient> = match StorageType::from_dsn(&db_settings.dsn) {
            #[cfg(feature = "dynamodb")]
            StorageType::DynamoDb => {
                debug!("Using Dynamodb");
                Box::new(DdbClientImpl::new(metrics.clone(), &db_settings)?)
            }
            #[cfg(feature = "bigtable")]
            StorageType::BigTable => {
                debug!("Using BigTable");
                let client = BigTableClientImpl::new(metrics.clone(), &db_settings)?;
                client.spawn_sweeper(Duration::from_secs(30));
                Box::new(client)
            }
            #[cfg(all(feature = "bigtable", feature = "dual"))]
            StorageType::Dual => {
                let client = DualClientImpl::new(metrics.clone(), &db_settings)?;
                client.spawn_sweeper(Duration::from_secs(30));
                Box::new(client)
            }
            _ => {
                debug!("No idea what {:?} is", &db_settings.dsn);
                return Err(ApiErrorKind::General(
                    "Invalid or Unsupported DSN specified".to_owned(),
                )
                .into());
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
                db.clone(),
            )
            .await?,
        );
        let apns_router = Arc::new(
            ApnsRouter::new(
                settings.apns.clone(),
                endpoint_url.clone(),
                metrics.clone(),
                db.clone(),
            )
            .await?,
        );
        #[cfg(feature = "adm")]
        let adm_router = Arc::new(AdmRouter::new(
            settings.adm.clone(),
            endpoint_url,
            http.clone(),
            metrics.clone(),
            db.clone(),
        )?);
        let app_state = AppState {
            metrics: metrics.clone(),
            settings,
            fernet,
            db,
            http,
            fcm_router,
            apns_router,
            #[cfg(feature = "adm")]
            adm_router,
        };

        spawn_pool_periodic_reporter(
            Duration::from_secs(10),
            app_state.db.clone(),
            app_state.metrics.clone(),
        );

        let server = HttpServer::new(move || {
            // These have a bad habit of being reset. Specify them explicitly.
            let cors = Cors::default()
                .allow_any_origin()
                .allow_any_header()
                .allowed_methods(vec![
                    actix_web::http::Method::DELETE,
                    actix_web::http::Method::GET,
                    actix_web::http::Method::POST,
                    actix_web::http::Method::PUT,
                ])
                .max_age(3600);
            App::new()
                // Actix 4 recommends wrapping structures wtih web::Data (internally an Arc)
                .app_data(Data::new(app_state.clone()))
                // Extractor configuration
                .app_data(web::PayloadConfig::new(app_state.settings.max_data_bytes))
                .app_data(web::JsonConfig::default().limit(app_state.settings.max_data_bytes))
                // Middleware
                .wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, ApiError::render_404))
                // Our modified Sentry wrapper which does some blocking of non-reportable errors.
                .wrap(SentryWrapper::<ApiError>::new(
                    metrics.clone(),
                    "api_error".to_owned(),
                ))
                .wrap(cors)
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
