#![warn(rust_2018_idioms)]
#![forbid(unsafe_code)]

#[macro_use]
extern crate slog_scope;

use std::{env, vec::Vec};

use actix_web::HttpServer;
use docopt::Docopt;
use serde::Deserialize;

use autoconnect_settings::{AppState, Settings};
use autoconnect_web::{build_app, config};
use autopush_common::{
    errors::{ApcErrorKind, Result},
    logging,
};

const USAGE: &str = "
Usage: autopush_rs [options]

Options:
    -h, --help                          Show this message.
    --config-connection=CONFIGFILE      Connection configuration file path.
    --config-shared=CONFIGFILE          Common configuration file path.
";

#[derive(Debug, Deserialize)]
struct Args {
    flag_config_connection: Option<String>,
    flag_config_shared: Option<String>,
}

#[actix_web::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let mut filenames = Vec::new();
    if let Some(shared_filename) = args.flag_config_shared {
        filenames.push(shared_filename);
    }
    if let Some(config_filename) = args.flag_config_connection {
        filenames.push(config_filename);
    }
    let settings =
        Settings::with_env_and_config_files(&filenames).map_err(ApcErrorKind::ConfigError)?;
    logging::init_logging(!settings.human_logs).expect("Logging failed to initialize");
    debug!("Starting up...");

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
    let app_state = AppState::from_settings(settings)?;
    app_state.init_and_spawn_megaphone_updater().await?;

    info!(
        "Starting autoconnect on port {} (router_port: {})",
        port, router_port
    );
    HttpServer::new(move || build_app!(app_state))
        .bind(("0.0.0.0", port))?
        .bind(("0.0.0.0", router_port))?
        .run()
        .await
        .map_err(|e| e.into())
        .map(|v| {
            info!("Shutting down autoconnect");
            v
        })
}
