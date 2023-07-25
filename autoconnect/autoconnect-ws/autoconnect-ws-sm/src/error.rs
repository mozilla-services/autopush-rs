use std::fmt;

use actix_ws::CloseCode;
use backtrace::Backtrace;

use autopush_common::{db::error::DbError, errors::ReportableError};

pub type NonStdBacktrace = backtrace::Backtrace;

/// WebSocket state machine errors
#[derive(Debug, thiserror::Error)]
pub struct SMError {
    pub kind: SMErrorKind,
    /// Avoid thiserror's automatic `std::backtrace::Backtrace` integration by
    /// not using the type name "Backtrace". The older `backtrace::Backtrace`
    /// is still preferred for Sentry integration:
    /// https://github.com/getsentry/sentry-rust/issues/600
    backtrace: NonStdBacktrace,
}

impl fmt::Display for SMError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

// Forward From impls to SMError from SMErrorKind. Because From is reflexive,
// this impl also takes care of From<SMErrorKind>.
impl<T> From<T> for SMError
where
    SMErrorKind: From<T>,
{
    fn from(item: T) -> Self {
        Self {
            kind: SMErrorKind::from(item),
            backtrace: Backtrace::new(),
        }
    }
}

impl SMError {
    pub fn close_code(&self) -> actix_ws::CloseCode {
        match self.kind {
            // TODO: applicable here?
            //SMErrorKind::InvalidMessage(_) => CloseCode::Invalid,
            SMErrorKind::UaidReset => CloseCode::Normal,
            _ => CloseCode::Error,
        }
    }

    pub fn invalid_message(description: String) -> Self {
        SMErrorKind::InvalidMessage(description).into()
    }
}

impl ReportableError for SMError {
    fn backtrace(&self) -> Option<&Backtrace> {
        Some(&self.backtrace)
    }

    fn is_sentry_event(&self) -> bool {
        matches!(
            self.kind,
            SMErrorKind::Database(_)
                | SMErrorKind::Internal(_)
                | SMErrorKind::Reqwest(_)
                | SMErrorKind::MakeEndpoint(_)
        )
    }

    fn metric_label(&self) -> Option<&'static str> {
        // TODO:
        None
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SMErrorKind {
    #[error("Database error: {0}")]
    Database(#[from] DbError),

    #[error("Invalid WebPush message: {0}")]
    InvalidMessage(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Reqwest error: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("UAID dropped")]
    UaidReset,

    #[error("Already connected to another node")]
    AlreadyConnected,

    #[error("New Client with the same UAID has connected to this node")]
    Ghost,

    #[error("Failed to generate endpoint: {0}")]
    MakeEndpoint(String),

    #[error("Client sent too many pings too often")]
    ExcessivePing,
}
