#![warn(rust_2018_idioms)]

#[macro_use]
extern crate slog_scope;

mod error;
mod logging;
mod metrics;
mod server;
mod settings;
mod tags;

use docopt::Docopt;
use sentry::internals::ClientInitGuard;
use serde::Deserialize;
use std::error::Error;

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
    logging::init_logging(!settings.human_logs).expect("Logging failed to initialize");
    debug!("Starting up...");

    // Configure sentry error capture
    let _sentry_guard = configure_sentry();

    // Run server...
    let server = server::Server::with_settings(settings)
        .await
        .expect("Could not start server");
    info!("Server started");
    server.await?;

    // Shutdown
    info!("Server closing");
    logging::reset_logging();
    Ok(())
}

fn configure_sentry() -> ClientInitGuard {
    let curl_transport_factory = |options: &sentry::ClientOptions| {
        Box::new(sentry::transports::CurlHttpTransport::new(&options))
            as Box<dyn sentry::internals::Transport>
    };
    let sentry = sentry::init(sentry::ClientOptions {
        transport: Box::new(curl_transport_factory),
        release: sentry::release_name!(),
        ..sentry::ClientOptions::default()
    });

    if sentry.is_enabled() {
        sentry::integrations::panic::register_panic_handler();
    }

    sentry
}
