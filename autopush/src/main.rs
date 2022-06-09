extern crate slog;
#[macro_use]
extern crate slog_scope;
#[macro_use]
extern crate serde_derive;

use std::{env, os::raw::c_int, thread};

use docopt::Docopt;

use autopush_common::errors::ApiResult;
use autopush_common::logging;

mod http;
mod megaphone;
//mod old_client;
mod aclient;
mod server;
mod routes;
mod settings;
mod user_agent;

use crate::server::{
    // old_server::AutopushServer,
    ServerOptions
};
use crate::server::aserver;
use crate::settings::Settings;

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

#[tokio::main]
async fn main() -> ApiResult<()> {
    //env_logger::init();
    let signal = notify(&[signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM])?;
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
    let settings = Settings::with_env_and_config_files(&filenames)?;
    logging::init_logging(!settings.human_logs).expect("Logging failed to initialize");
    // Setup the AWS env var if it was set
    if let Some(ref ddb_local) = settings.aws_ddb_endpoint {
        env::set_var("AWS_LOCAL_DYNAMODB", ddb_local);
    }
    info!("Server starting on port {}", &settings.port);
    trace!(
        "message {:?}, router: {:?}",
        &settings.message_tablename,
        &settings.router_tablename
    );
    let server_opts = ServerOptions::from_settings(settings)?;
    let server = aserver::Server::from_opts(server_opts)?;
    server.start();
    signal.recv().unwrap();
    info!("Server closing");
    server.stop().map_err(|| "Failed to shutdown properly")
}

/// Create a new channel subscribed to the given signals
fn notify(signals: &[c_int]) -> ApiResult<crossbeam_channel::Receiver<c_int>> {
    let (s, r) = crossbeam_channel::bounded(100);
    let mut signals = signal_hook::iterator::Signals::new(signals)?;
    thread::spawn(move || {
        for signal in signals.forever() {
            if s.send(signal).is_err() {
                break;
            }
        }
    });
    Ok(r)
}
