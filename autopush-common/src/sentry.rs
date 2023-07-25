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
/// access its backtrace.
pub fn event_from_error<E>(err: &E) -> sentry::protocol::Event<'static>
where
    E: ReportableError + 'static,
{
    let mut exceptions = vec![exception_from_error_with_backtrace(err)];

    let mut source = err.source();
    while let Some(err) = source {
        let exception = if let Some(err) = err.downcast_ref::<E>() {
            exception_from_error_with_backtrace(err)
        } else {
            exception_from_error(err)
        };
        exceptions.push(exception);
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
/// Based moreso on sentry_failure's `exception_from_single_fail`.
fn exception_from_error_with_backtrace(err: &dyn ReportableError) -> sentry::protocol::Exception {
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
