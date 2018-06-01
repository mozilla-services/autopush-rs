#[macro_use]
extern crate serde_derive;
extern crate autopush;
extern crate chan_signal;
extern crate docopt;

use std::env;

use chan_signal::Signal;
use docopt::Docopt;

use autopush::errors::{Result, ResultExt};
use autopush::server::{AutopushServer, ServerOptions};
use autopush::settings::Settings;

const USAGE: &'static str = "
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
    let signal = chan_signal::notify(&[Signal::INT, Signal::TERM]);
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
    server.stop().chain_err(|| "Failed to shutdown properly")
}
