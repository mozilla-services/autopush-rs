use std::io;

use gethostname::gethostname;
use slog::{self, Drain};
use slog_mozlog_json::MozLogJson;

use crate::errors::Result;

pub fn init_logging(json: bool, name: &str, version: &str) -> Result<()> {
    let logger = if json {
        let hostname = gethostname().to_string_lossy().to_string();

        let drain = MozLogJson::new(io::stdout())
            .logger_name(format!("{name}-{version}"))
            .msg_type(format!("{name}:log"))
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

/// Return parallelism/number of CPU information to log at startup
pub fn parallelism_banner() -> String {
    format!(
        "available_parallelism: {:?} num_cpus: {} num_cpus (phys): {}",
        std::thread::available_parallelism(),
        num_cpus::get(),
        num_cpus::get_physical()
    )
}
