use std::{io, sync::OnceLock, time::Duration};

use gethostname::gethostname;
use slog::{self, Drain};
use slog_mozlog_json::MozLogJson;

use crate::errors::Result;

static EC2_INSTANCE_ID: OnceLock<Option<String>> = OnceLock::new();

pub fn init_logging(json: bool) -> Result<()> {
    let logger = if json {
        let ec2_instance_id = EC2_INSTANCE_ID.get_or_init(|| get_ec2_instance_id().ok());
        let hostname = ec2_instance_id
            .clone()
            .unwrap_or_else(|| gethostname().to_string_lossy().to_string());
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
/// NOTE: This issues a blocking web request
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
