#![warn(rust_2018_idioms)]
#![forbid(unsafe_code)]

#[macro_use]
extern crate slog_scope;

mod auth;
mod error;
mod extractors;
mod headers;
mod metrics;
mod routers;
mod routes;
mod server;
mod settings;

use docopt::Docopt;
use serde::Deserialize;
use std::error::Error;

use autopush_common::logging;

const USAGE: &str = "
Usage: autoendpoint [options]

Options:
    -h, --help              Show this message
    --config=CONFIGFILE     AutoEndpoint configuration file path.
";

#[derive(Debug, Deserialize)]
struct Args {
    flag_config: Option<String>,
}

#[actix_rt::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let settings = settings::Settings::with_env_and_config_file(&args.flag_config)?;
    let host_port = format!("{}:{}", &settings.host, &settings.port);
    logging::init_logging(!settings.human_logs).expect("Logging failed to initialize");
    debug!("Starting up autoendpoint...");

    let _sentry = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        attach_stacktrace: true,
        ..autopush_common::sentry::client_options()
    });

    // Run server...
    let server = server::Server::with_settings(settings)
        .await
        .expect("Could not start server");
    info!("Server started: {}", host_port);
    server.await?;

    // Shutdown
    info!("Server closing");
    logging::reset_logging();
    Ok(())
}
