use std::io;

use crate::errors::Result;

use mozsvc_common::{aws::get_ec2_instance_id, get_hostname};
use slog::{self, Drain};
use slog_mozlog_json::MozLogJson;

pub fn init_logging(json: bool) -> Result<()> {
    let logger = if json {
        let hostname = get_ec2_instance_id()
            .map(str::to_owned)
            .or_else(|| {
                Some(
                    // If we can't get the hostname, we can't log.
                    // Fail fast so that SRE can see the error.
                    get_hostname()
                        .expect("Could not get logging hostname")
                        .to_str()
                        .expect("Could not parse hostname")
                        .to_owned(),
                )
            })
            .expect("Could not get logging hostname");

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
