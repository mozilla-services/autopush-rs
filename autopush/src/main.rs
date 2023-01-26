extern crate slog;
#[macro_use]
extern crate slog_scope;
#[macro_use]
extern crate serde_derive;

use std::{env, os::raw::c_int, thread};

use docopt::Docopt;

use autopush_common::errors::{ApcErrorKind, Result};

mod client;
mod http;
mod megaphone;
mod server;
mod settings;
mod user_agent;

use crate::server::{AutopushServer, ServerOptions};
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

fn main() -> Result<()> {
    env_logger::init();
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
    // Setup the AWS env var if it was set
    if let Some(ref ddb_local) = settings.aws_ddb_endpoint {
        env::set_var("AWS_LOCAL_DYNAMODB", ddb_local);
    }
    let server_opts = ServerOptions::from_settings(settings)?;
    let server = AutopushServer::new(server_opts);
    server.start();
    signal.recv().unwrap();
    server
        .stop()
        .map_err(|_e| ApcErrorKind::GeneralError("Failed to shutdown properly".into()).into())
}

/// Create a new channel subscribed to the given signals
fn notify(signals: &[c_int]) -> Result<crossbeam_channel::Receiver<c_int>> {
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
