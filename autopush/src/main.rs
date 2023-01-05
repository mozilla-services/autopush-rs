extern crate slog;
#[macro_use]
extern crate slog_scope;
#[macro_use]
extern crate serde_derive;

use std::{env, os::raw::c_int, thread};
use std::time::Duration;

use docopt::Docopt;

use autopush_common::errors::{ApiResult, ApiError};
use futures::{Future, future::Either};
use tokio_core::reactor::{Handle, Timeout};

mod client;
mod http;
mod megaphone;
mod server;
mod settings;
mod user_agent;

use crate::server::{AutopushServer, ServerOptions};
use crate::settings::Settings;

pub type MyFuture<T> = Box<dyn Future<Item = T, Error = autopush_common::errors::ApiError>>;

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

fn main() -> ApiResult<()> {
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
    server.stop().chain_err(|| "Failed to shutdown properly")
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


/// Convenience future to time out the resolution of `f` provided within the
/// duration provided.
///
/// If the `dur` is `None` then the returned future is equivalent to `f` (no
/// timeout) and otherwise the returned future will cancel `f` and resolve to an
/// error if the `dur` timeout elapses before `f` resolves.
pub fn timeout<F>(f: F, dur: Option<Duration>, handle: &Handle) -> MyFuture<F::Item>
where
    F: Future + 'static,
    F::Error: Into<ApiError>,
{
    let dur = match dur {
        Some(dur) => dur,
        None => return Box::new(f.map_err(|e| e.into())),
    };
    let timeout = Timeout::new(dur, handle).into_future().flatten();
    Box::new(f.select2(timeout).then(|res| match res {
        Ok(Either::A((item, _timeout))) => Ok(item),
        Err(Either::A((e, _timeout))) => Err(e.into()),
        Ok(Either::B(((), _item))) => Err("timed out".into()),
        Err(Either::B((e, _item))) => Err(e.into()),
    }))
}
