use std::{io, sync::OnceLock, time::Duration};

use gethostname::gethostname;
use serde_derive::{Deserialize, Serialize};
use slog::{self, Drain};
use slog_mozlog_json::MozLogJson;

use crate::errors::Result;

static INSTANCE_ID: OnceLock<Option<String>> = OnceLock::new();

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum Platform {
    AWS,
    GCP,
    UNKNOWN,
}

impl From<&str> for Platform {
    fn from(string: &str) -> Platform {
        match string.to_ascii_lowercase().as_str() {
            "aws" => Platform::AWS,
            "gcp" => Platform::GCP,
            _ => Platform::UNKNOWN,
        }
    }
}

/// Initialize logging.
///
/// This will generate either mozilla standardized JSON output or a
/// more "human readable" form. It also uses the provided hostname
/// identifier as part of the standardized output.
pub fn init_logging(json: bool, hostname: String) -> Result<()> {
    let logger = if json {
        let drain = MozLogJson::new(io::stdout())
            .logger_name(format!(
                "{}-{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))
            .msg_type(format!("{}:log", env!("CARGO_PKG_NAME")))
            .hostname(hostname)
            .build()
            .fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        slog::Logger::root(drain, slog_o!())
    } else {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain).build().fuse();
        slog::Logger::root(drain, slog_o!())
    };
    // XXX: cancel slog_scope's NoGlobalLoggerSet for now, it's difficult to
    // prevent it from potentially panicing during tests. reset_logging resets
    // the global logger during shutdown anyway:
    // https://github.com/slog-rs/slog/issues/169
    slog_scope::set_global_logger(logger).cancel_reset();
    slog_stdlog::init().ok();
    Ok(())
}

pub fn reset_logging() {
    let logger = slog::Logger::root(slog::Discard, o!());
    slog_scope::set_global_logger(logger).cancel_reset();
}

/// Initialize logging to `slog_term::TestStdoutWriter` for tests
///
/// Note: unfortunately this disables slog's `TermDecorator` (which can't be
/// captured by cargo test) color output
pub fn init_test_logging() {
    let decorator = slog_term::PlainSyncDecorator::new(slog_term::TestStdoutWriter);
    let drain = std::sync::Mutex::new(slog_term::FullFormat::new(decorator).build()).fuse();
    let logger = slog::Logger::root(drain, slog::o!());
    slog_scope::set_global_logger(logger).cancel_reset();
    slog_stdlog::init().ok();
}

/// Fetch the EC2 instance-id
///
/// NOTE: This issues a blocking web request and will cause the app to panic unless the
/// async context has been initialized. Since it is HIGHLY likely that this function will
/// be called ouside of the async context, we use the thread primatives to encapsulate this
/// call.
pub fn get_ec2_instance_id() -> Result<Option<String>> {
    let instance_id = INSTANCE_ID.get_or_init(|| {
        std::thread::spawn(|| {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(1))
                .build()?;
            client
                .get("http://169.254.169.254/latest/meta-data/instance-id")
                .send()?
                .error_for_status()?
                .text()
        })
        .join()
        .unwrap()
        .ok()
    });
    Ok(instance_id.clone())
}

// It may be possible to reuse the above to get the ephemeral host ID on GCP.
// SRE has not yet requested we do that.

/// Use the run time specific Hostname identifier, falling back to the system level hostname
///
/// e.g. for `autoconnect` this will attempt to look for `AUTOCONNECT_HOSTNAME`, and if that
/// is not defined, will use the locally specified hostname.
pub fn get_default_hostname(prefix: &str) -> String {
    std::env::var(format!("{}_HOSTNAME", prefix.to_uppercase()))
        .unwrap_or_else(|_| gethostname().to_string_lossy().to_string())
}
