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
