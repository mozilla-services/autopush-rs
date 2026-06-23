//! TLS / rustls process setup.

/// Install a process-level rustls [`CryptoProvider`] so that rustls 0.23 has an
/// unambiguous default.
///
/// In the bigtable build our dependency graph unifies onto a single `rustls`
/// 0.23 build that has *both* the `ring` and `aws-lc-rs` provider features
/// compiled in (`ring` via tonic's `tls-ring` and gcp_auth; `aws-lc-rs` via
/// reqwest). When more than one provider is present, any library that builds a
/// rustls config through the default path — e.g. reqwest, on the actix arbiter
/// threads at startup — panics because rustls can't pick a provider. (tonic
/// itself doesn't panic: it falls back to `ring`, but it adopts whatever
/// default we install here.)
///
/// We install `aws-lc-rs` because reqwest pulls it into every build, so this
/// never introduces a second provider into the otherwise single-provider
/// redis/postgres builds.
///
/// Call this once, as early as possible in `main()`, before any TLS use. It is
/// idempotent: a second call (or a provider installed by another crate) is
/// ignored.
///
/// [`CryptoProvider`]: rustls::crypto::CryptoProvider
pub fn install_crypto_provider() {
    if rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .is_err()
    {
        // A provider was already installed for this process; nothing to do.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the fix for the ambiguous-`CryptoProvider` startup panic.
    ///
    /// In any build that compiles in more than one rustls provider (the
    /// `bigtable` build pulls `ring` via tonic *and* `aws-lc-rs` via reqwest),
    /// `rustls::ClientConfig::builder()` resolves `CryptoProvider::get_default()`
    /// and panics with "Could not automatically determine the process-level
    /// CryptoProvider" unless a default has been installed first.
    ///
    /// This asserts that [`install_crypto_provider`] makes that resolution
    /// succeed, so the clients that use rustls's default-provider path (reqwest,
    /// sqlx) no longer panic. tonic doesn't hit this path — it falls back to
    /// `ring` — but it adopts whatever default we install for the bigtable
    /// connection.
    #[test]
    fn install_resolves_crypto_provider() {
        install_crypto_provider();

        // Idempotent: a second call must not panic.
        install_crypto_provider();

        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "no process-level CryptoProvider after install"
        );

        // The exact call that panics pre-fix when multiple providers are
        // compiled in. It must not panic now.
        let _ = rustls::ClientConfig::builder();
    }
}
