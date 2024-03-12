#![warn(rust_2018_idioms)]
#![forbid(unsafe_code)]

#[macro_use]
extern crate slog_scope;

use std::{env, time::Duration, vec::Vec};

use actix_http::HttpService;
use actix_server::Server;
use actix_service::map_config;
use actix_web::dev::AppConfig;
use docopt::Docopt;
use serde::Deserialize;

use autoconnect_settings::{AppState, Settings};
use autoconnect_web::{build_app, config, config_router};
use autopush_common::{
    db::spawn_pool_periodic_reporter,
    errors::{ApcErrorKind, Result},
    logging,
};

const USAGE: &str = "
Usage: autoconnect [options]

Options:
    -h, --help                          Show this message.
    --config=CONFIGFILE                 Connection configuration file path.
";

#[derive(Debug, Deserialize)]
struct Args {
    flag_config: Option<String>,
}

#[actix_web::main]
async fn main() -> Result<()> {
    env_logger::init();
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let mut filenames = Vec::new();
    if let Some(config_filename) = args.flag_config {
        filenames.push(config_filename);
    }
    let settings =
        Settings::with_env_and_config_files(&filenames).map_err(ApcErrorKind::ConfigError)?;
    logging::init_logging(
        !settings.human_logs,
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION"),
    )
    .expect("Logging failed to initialize");
    debug!("Starting up autoconnect...");

    // Sentry requires the environment variable "SENTRY_DSN".
    if env::var("SENTRY_DSN")
        .unwrap_or_else(|_| "".to_owned())
        .is_empty()
    {
        print!("SENTRY_DSN not set. Logging disabled.");
    }

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        session_mode: sentry::SessionMode::Request, // new session per request
        auto_session_tracking: true,
        ..autopush_common::sentry::client_options()
    });

    let port = settings.port;
    let router_port = settings.router_port;
    let actix_max_connections = settings.actix_max_connections;
    let actix_workers = settings.actix_workers;
    let app_state = AppState::from_settings(settings)?;
    app_state.init_and_spawn_megaphone_updater().await?;
    spawn_pool_periodic_reporter(
        Duration::from_secs(10),
        app_state.db.clone(),
        app_state.metrics.clone(),
    );

    info!(
        "Starting autoconnect on port: {} router_port: {} ({})",
        port,
        router_port,
        logging::parallelism_banner()
    );

    let router_app_state = app_state.clone();
    let mut builder = Server::build()
        .bind("autoconnect", ("0.0.0.0", port), move || {
            let app = build_app!(app_state, config);
            HttpService::build()
                // XXX: AppConfig::default() does *not* have correct values
                // https://github.com/actix/actix-web/issues/3180
                .finish(map_config(app, |_| AppConfig::default()))
                .tcp()
        })?
        .bind("autoconnect-router", ("0.0.0.0", router_port), move || {
            let app = build_app!(router_app_state, config_router);
            HttpService::build()
                // XXX:
                .finish(map_config(app, |_| AppConfig::default()))
                .tcp()
        })?;
    if let Some(max_connections) = actix_max_connections {
        builder = builder.max_concurrent_connections(max_connections);
    }
    if let Some(workers) = actix_workers {
        builder = builder.workers(workers);
    }
    builder.run().await?;

    info!("Shutting down autoconnect");
    Ok(())
}
