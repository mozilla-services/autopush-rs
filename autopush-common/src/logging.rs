use std::io;
#[cfg(feature = "aws")]
use std::{sync::OnceLock, time::Duration};

use gethostname::gethostname;
use slog::{self, Drain};
use slog_mozlog_json::MozLogJson;

use crate::errors::Result;

#[cfg(featre = "aws")]
static EC2_INSTANCE_ID: OnceLock<Option<String>> = OnceLock::new();

pub fn init_logging(json: bool, name: &str, version: &str) -> Result<()> {
    let logger = if json {
        let hostname = {
            #[cfg(feature = "aws")]
            let result = ec2_instance_id();
            #[cfg(not(feature = "aws"))]
            let result = gethostname().to_string_lossy().to_string();
            result
        };
        let drain = MozLogJson::new(io::stdout())
            .logger_name(format!("{}-{}", name, version))
            .msg_type(format!("{}:log", name))
            .hostname(hostname)
            .build()
            .fuse();
        let drain = slog_envlogger::new(drain);
        let drain = slog_async::Async::new(drain).build().fuse();
        slog::Logger::root(drain, slog_o!())
    } else {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_envlogger::new(drain);
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

#[cfg(feature = "aws")]
fn ec2_instance_id() -> String {
    let ec2_instance_id = EC2_INSTANCE_ID.get_or_init(|| get_ec2_instance_id().ok());
    ec2_instance_id
        .clone()
        .unwrap_or_else(|| gethostname().to_string_lossy().to_string())
}
/// Fetch the EC2 instance-id
///
/// NOTE: This issues a blocking web request
#[cfg(feature = "aws")]
fn get_ec2_instance_id() -> reqwest::Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()?;
    client
        .get("http://169.254.169.254/latest/meta-data/instance-id")
        .send()?
        .error_for_status()?
        .text()
}
