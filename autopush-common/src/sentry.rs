use std::error::Error;

use crate::errors::ReportableError;

/// Return a `sentry::::ClientOptions` w/ the `debug-images` integration
/// disabled
pub fn client_options() -> sentry::ClientOptions {
    // debug-images conflicts w/ our debug = 1 rustc build option:
    // https://github.com/getsentry/sentry-rust/issues/574
    let mut opts = sentry::apply_defaults(sentry::ClientOptions::default());
    opts.integrations.retain(|i| i.name() != "debug-images");
    opts.default_integrations = false;
    opts
}

/// Custom `sentry::event_from_error` for `ReportableError`
///
/// `std::error::Error` doesn't support backtraces, thus `sentry::event_from_error`
/// doesn't either. This function works against `ReportableError` instead to
/// extract backtraces from it and its chain of `reportable_source's.
///
/// A caveat of this function is that it cannot extract
/// `ReportableError`s/backtraces that occur in a chain after a
/// `std::error::Error` occurs: as `std::error::Error::source` only allows
/// downcasting to a concrete type, not `dyn ReportableError`.
pub fn event_from_error(
    mut reportable_err: &dyn ReportableError,
) -> sentry::protocol::Event<'static> {
    let mut exceptions = vec![];

    // Gather reportable_source()'s for their backtraces
    loop {
        exceptions.push(exception_from_reportable_error(reportable_err));
        reportable_err = match reportable_err.reportable_source() {
            Some(reportable_err) => reportable_err,
            None => break,
        };
    }

    // Then fallback to source() for remaining Errors
    let mut source = reportable_err.source();
    while let Some(err) = source {
        exceptions.push(exception_from_error(err));
        source = err.source();
    }

    exceptions.reverse();
    sentry::protocol::Event {
        exception: exceptions.into(),
        level: sentry::protocol::Level::Error,
        ..Default::default()
    }
}

/// Custom `exception_from_error` support function for `ReportableError`
///
/// Based moreso on sentry_failure's `exception_from_single_fail`. Includes a
/// stacktrace if available.
fn exception_from_reportable_error(err: &dyn ReportableError) -> sentry::protocol::Exception {
    let mut exception = exception_from_error(err);
    if let Some(backtrace) = err.backtrace() {
        exception.stacktrace = sentry_backtrace::backtrace_to_stacktrace(backtrace)
    }
    exception
}

/// Exact copy of sentry's unfortunately private `exception_from_error`
fn exception_from_error<E>(err: &E) -> sentry::protocol::Exception
where
    E: Error + ?Sized,
{
    let dbg = format!("{:?}", err);
    sentry::protocol::Exception {
        ty: sentry::parse_type_from_debug(&dbg).to_owned(),
        value: Some(err.to_string()),
        ..Default::default()
    }
}
