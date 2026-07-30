use std::io;

use gethostname::gethostname;
use slog::{self, Drain};
use slog_mozlog_json::MozLogJson;

use crate::errors::Result;

/// Default number of records slog-async buffers before it begins dropping them.
///
/// slog-async's own default of 128 is trivially overrun: the consumer only needs
/// to stall briefly (scheduling, a slow write) for the buffer to fill, and each
/// dropped record then produces an ERROR level "channel overflow" report of its
/// own, converting filtered-out records into unfilterable noise. Sized instead
/// for seconds of headroom at a high logging rate; ~110 bytes per slot is
/// allocated up front, so this costs roughly 2MB resident.
pub const DEFAULT_LOG_CHAN_SIZE: usize = 20_000;

/// Initialize logging.
///
/// `chan_size` is the slog-async buffer depth; see [`DEFAULT_LOG_CHAN_SIZE`]. A
/// value of 0 is treated as the default: crossbeam would otherwise give us a
/// rendezvous channel, blocking every logging call until the consumer picks the
/// record up.
pub fn init_logging(json: bool, chan_size: usize, name: &str, version: &str) -> Result<()> {
    let chan_size = if chan_size == 0 {
        DEFAULT_LOG_CHAN_SIZE
    } else {
        chan_size
    };
    // NOTE: `slog_envlogger` (the RUST_LOG filter) must be the *outermost*
    // drain, wrapping `slog_async`. Nested inside it instead, every record is
    // queued to the async channel before anyone checks its level, so RUST_LOG
    // can't relieve channel pressure and filtered records still cause overflow.
    let (logger, filter_level) = if json {
        let hostname = gethostname().to_string_lossy().to_string();

        let drain = MozLogJson::new(io::stdout())
            .logger_name(format!("{name}-{version}"))
            .msg_type(format!("{name}:log"))
            .hostname(hostname)
            .build()
            .fuse();
        let drain = slog_async::Async::new(drain)
            .chan_size(chan_size)
            .build()
            .fuse();
        let drain = slog_envlogger::new(drain);
        let filter_level = slog_envlogger::EnvLogger::filter(&drain);
        (slog::Logger::root(drain, slog_o!()), filter_level)
    } else {
        let decorator = slog_term::TermDecorator::new().build();
        let drain = slog_term::FullFormat::new(decorator).build().fuse();
        let drain = slog_async::Async::new(drain)
            .chan_size(chan_size)
            .build()
            .fuse();
        let drain = slog_envlogger::new(drain);
        let filter_level = slog_envlogger::EnvLogger::filter(&drain);
        (slog::Logger::root(drain, slog_o!()), filter_level)
    };
    // XXX: cancel slog_scope's NoGlobalLoggerSet for now, it's difficult to
    // prevent it from potentially panicing during tests. reset_logging resets
    // the global logger during shutdown anyway:
    // https://github.com/slog-rs/slog/issues/169
    slog_scope::set_global_logger(logger).cancel_reset();
    // Register the `log` -> `slog` bridge, then set `log`'s ceiling separately:
    // `init_with_level` takes a `log::Level`, which can't express "off".
    slog_stdlog::init_with_level(log::Level::Error).ok();
    log::set_max_level(log_max_level(filter_level));
    Ok(())
}

/// Translate the `RUST_LOG` filter's maximum level into a ceiling for the `log`
/// crate.
///
/// Our own `slog` macros reach the logger directly, so this governs only records
/// arriving via `log` -- that is, our dependencies. hyper/h2/tonic/tower emit
/// `tracing` events and, with no `tracing` subscriber installed, those fall
/// through to `log`; h2 in particular traces per HTTP/2 frame.
///
/// `slog_stdlog::init` would set the ceiling to TRACE, admitting all of it.
/// Deriving the ceiling from the filter's own maximum instead means a record no
/// directive could ever accept is rejected by `log::max_level()` before it is
/// built, rather than being constructed and then dropped by the filter. Records
/// between this ceiling and a narrower per-module directive still reach the
/// filter, which remains the authority on what is actually logged.
fn log_max_level(filter: slog::FilterLevel) -> log::LevelFilter {
    match filter {
        slog::FilterLevel::Off => log::LevelFilter::Off,
        // `log` has no Critical; Error is the nearest ceiling that admits it.
        slog::FilterLevel::Critical | slog::FilterLevel::Error => log::LevelFilter::Error,
        slog::FilterLevel::Warning => log::LevelFilter::Warn,
        slog::FilterLevel::Info => log::LevelFilter::Info,
        slog::FilterLevel::Debug => log::LevelFilter::Debug,
        slog::FilterLevel::Trace => log::LevelFilter::Trace,
    }
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
