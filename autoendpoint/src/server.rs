//! Main application server
#![forbid(unsafe_code)]
use std::sync::atomic::{AtomicUsize, Ordering};
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
#[cfg(feature = "postgres")]
use autopush_common::db::postgres::PgClientImpl;
#[cfg(feature = "redis")]
use autopush_common::db::redis::RedisClientImpl;
#[cfg(feature = "reliable_report")]
use autopush_common::reliability::PushReliability;
use autopush_common::{
    db::{client::DbClient, spawn_pool_periodic_reporter, DbSettings, StorageType},
    metric_name::MetricName,
    middleware::sentry::SentryWrapper,
};

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::metrics;
#[cfg(feature = "stub")]
use crate::routers::stub::router::StubRouter;
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
use crate::settings::VapidTracker;

#[derive(Clone)]
pub struct AppState {
    /// Server Data
    pub metrics: Arc<StatsdClient>,
    pub settings: Settings,
    pub fernet: MultiFernet,
    pub db: Box<dyn DbClient>,
    pub http: reqwest::Client,
    // requests that are currently being exchanged between nodes.
    pub in_flight_requests: Arc<AtomicUsize>,
    // subscription updates that are currently being processed.
    pub in_process_subscription_updates: Arc<AtomicUsize>,
    pub fcm_router: Arc<FcmRouter>,
    pub apns_router: Arc<ApnsRouter>,
    #[cfg(feature = "stub")]
    pub stub_router: Arc<StubRouter>,
    #[cfg(feature = "reliable_report")]
    pub reliability: Arc<PushReliability>,
    pub reliability_filter: VapidTracker,
}

#[cfg(test)]
impl AppState {
    /// build a fake AppState with a MockDbClient and default settings, for use in tests.
    pub(crate) async fn test_default(mock_db: autopush_common::db::mock::MockDbClient) -> Self {
        let settings = Settings {
            auth_keys: "HJVPy4ZwF4Yz_JdvXTL8hRcwIhv742vC60Tg5Ycrvw8=".to_owned(),
            ..Default::default()
        };
        let metrics = Arc::new(crate::metrics::Metrics::sink());
        let db = mock_db.into_boxed_arc();
        #[cfg(feature = "reliable_report")]
        let reliability =
            Arc::new(PushReliability::new(&None, db.box_clone(), &metrics.clone(), 0).unwrap());
        let fernet = settings.make_fernet();
        Self {
            metrics: metrics.clone(),
            settings,
            fernet,
            db: db.clone(),
            http: reqwest::Client::new(),
            in_flight_requests: Arc::new(AtomicUsize::new(0)),
            in_process_subscription_updates: Arc::new(AtomicUsize::new(0)),
            fcm_router: Arc::new(
                FcmRouter::new(
                    crate::routers::fcm::settings::FcmSettings::default(),
                    url::Url::parse("https://example.com").unwrap(),
                    reqwest::Client::new(),
                    metrics.clone(),
                    db.clone(),
                    #[cfg(feature = "reliable_report")]
                    reliability.clone(),
                )
                .await
                .unwrap(),
            ),
            apns_router: Arc::new(
                ApnsRouter::new(
                    crate::routers::apns::settings::ApnsSettings::default(),
                    url::Url::parse("https://example.com").unwrap(),
                    metrics.clone(),
                    db.clone(),
                    #[cfg(feature = "reliable_report")]
                    reliability.clone(),
                )
                .await
                .unwrap(),
            ),
            #[cfg(feature = "stub")]
            stub_router: Arc::new(
                StubRouter::new(crate::routers::stub::settings::StubSettings::default()).unwrap(),
            ),
            #[cfg(feature = "reliable_report")]
            reliability,
            reliability_filter: VapidTracker(Vec::new()),
        }
    }
}

pub struct Server;

impl Server {
    pub async fn with_settings(settings: Settings) -> ApiResult<dev::Server> {
        let metrics = Arc::new(metrics::metrics_from_settings(&settings)?);
        let bind_address = format!("{}:{}", settings.host, settings.port);
        let fernet = settings.make_fernet();
        let endpoint_url = settings.endpoint_url();
        let reliability_filter = VapidTracker(
            settings
                .tracking_keys()
                .map_err(|e| ApiErrorKind::General(format!("Configuration Error: {e}")))?,
        );
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
            #[cfg(feature = "bigtable")]
            StorageType::BigTable => {
                debug!("Using BigTable");
                let client = BigTableClientImpl::new(metrics.clone(), &db_settings)?;
                client.spawn_sweeper(Duration::from_secs(30));
                Box::new(client)
            }
            #[cfg(feature = "postgres")]
            StorageType::Postgres => {
                debug!("Using Postgres");
                let client = PgClientImpl::new(metrics.clone(), &db_settings)?;
                // client.spawn_sweeper(Duration::from_secs(30));
                Box::new(client)
            }
            #[cfg(feature = "redis")]
            StorageType::Redis => Box::new(RedisClientImpl::new(metrics.clone(), &db_settings)?),
            _ => {
                debug!("No idea what {:?} is", &db_settings.dsn);
                return Err(ApiErrorKind::General(
                    "Invalid or Unsupported DSN specified".to_owned(),
                )
                .into());
            }
        };
        #[cfg(feature = "reliable_report")]
        let reliability = Arc::new(
            PushReliability::new(
                &settings.reliability_dsn,
                db.clone(),
                &metrics,
                settings.reliability_retry_count,
            )
            .map_err(|e| {
                ApiErrorKind::General(format!("Could not initialize Reliability Report: {e:?}"))
            })?,
        );
        let http = reqwest::ClientBuilder::new()
            .connect_timeout(Duration::from_millis(settings.connection_timeout_millis))
            .timeout(Duration::from_millis(settings.request_timeout_millis))
            .pool_max_idle_per_host(settings.pool_max_idle_per_host)
            .pool_idle_timeout(Duration::from_secs(settings.pool_idle_timeout_secs))
            .build()
            .expect("Could not generate request client");
        let fcm_router = Arc::new(
            FcmRouter::new(
                settings.fcm.clone(),
                endpoint_url.clone(),
                http.clone(),
                metrics.clone(),
                db.clone(),
                #[cfg(feature = "reliable_report")]
                reliability.clone(),
            )
            .await?,
        );
        let apns_router = Arc::new(
            ApnsRouter::new(
                settings.apns.clone(),
                endpoint_url.clone(),
                metrics.clone(),
                db.clone(),
                #[cfg(feature = "reliable_report")]
                reliability.clone(),
            )
            .await?,
        );
        #[cfg(feature = "stub")]
        let stub_router = Arc::new(StubRouter::new(settings.stub.clone())?);
        let in_flight_requests = Arc::new(AtomicUsize::new(0));
        let in_process_subscription_updates = Arc::new(AtomicUsize::new(0));
        let app_state = AppState {
            metrics: metrics.clone(),
            settings,
            fernet,
            db,
            http,
            in_flight_requests: in_flight_requests.clone(),
            in_process_subscription_updates: in_process_subscription_updates.clone(),
            fcm_router,
            apns_router,
            #[cfg(feature = "stub")]
            stub_router,
            #[cfg(feature = "reliable_report")]
            reliability,
            reliability_filter,
        };

        spawn_pool_periodic_reporter(
            Duration::from_secs(10),
            app_state.db.clone(),
            app_state.metrics.clone(),
        );

        // Periodically report in-flight request gauge
        {
            let metrics = app_state.metrics.clone();
            let in_flight = in_flight_requests;
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(10));
                loop {
                    interval.tick().await;
                    let count = in_flight.load(Ordering::Relaxed);
                    if let Err(e) = cadence::Gauged::gauge(
                        metrics.as_ref(),
                        MetricName::InFlightNodeRequests.as_ref(),
                        count as u64,
                    ) {
                        debug!("Failed to report in-flight metric: {}", e);
                    }
                }
            });
        }

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
            let app = App::new()
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
                    app_state.settings.disable_sentry,
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
                .service(web::resource("/__version__").route(web::get().to(version_route)));
            #[cfg(feature = "reliable_report")]
            // Note: Only the endpoint returns the Prometheus "/metrics" collection report. This report contains all metrics for both
            // connection and endpoint, inclusive. It is served here mostly for simplicity's sake (since the endpoint handles more
            // HTTP requests than the connection server, and this will simplify metric collection and reporting.)
            let app = app.service(
                web::resource("/metrics")
                    .route(web::get().to(crate::routes::reliability::report_handler)),
            );
            app
        })
        .bind(bind_address)?
        .run();

        Ok(server)
    }
}
