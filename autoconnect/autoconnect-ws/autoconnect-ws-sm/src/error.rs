use std::{error::Error, fmt};

use actix_ws::CloseCode;
use backtrace::Backtrace;

use autoconnect_common::protocol::{ClientMessage, MessageType, ServerMessage};
use autopush_common::{db::error::DbError, errors::ApcError, errors::ReportableError};

/// Trait for types that can provide a MessageType
pub trait MessageTypeProvider {
    /// Returns the message type of this object
    fn message_type(&self) -> MessageType;
}

impl MessageTypeProvider for ClientMessage {
    fn message_type(&self) -> MessageType {
        self.message_type()
    }
}

impl MessageTypeProvider for ServerMessage {
    fn message_type(&self) -> MessageType {
        self.message_type()
    }
}

/// WebSocket state machine errors
#[derive(Debug)]
pub struct SMError {
    pub kind: SMErrorKind,
    backtrace: Option<Backtrace>,
}

impl fmt::Display for SMError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl Error for SMError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.kind.source()
    }
}

// Forward From impls to SMError from SMErrorKind. Because From is reflexive,
// this impl also takes care of From<SMErrorKind>.
impl<T> From<T> for SMError
where
    SMErrorKind: From<T>,
{
    fn from(item: T) -> Self {
        let kind = SMErrorKind::from(item);
        let backtrace = (kind.is_sentry_event() && kind.capture_backtrace()).then(Backtrace::new);
        Self { kind, backtrace }
    }
}

impl SMError {
    pub fn close_code(&self) -> actix_ws::CloseCode {
        match self.kind {
            SMErrorKind::UaidReset => CloseCode::Normal,
            _ => CloseCode::Error,
        }
    }

    pub fn invalid_message(description: String) -> Self {
        SMErrorKind::InvalidMessage(description).into()
    }

    /// Creates an invalid message error for an expected message type
    pub fn expected_message_type(expected: MessageType) -> Self {
        SMErrorKind::InvalidMessage(expected.expected_msg()).into()
    }

    /// Validates a message is of the expected type, returning an error if not
    pub fn validate_message_type<T>(expected: MessageType, msg: &T) -> Result<(), Self>
    where
        T: MessageTypeProvider,
    {
        if msg.message_type() == expected {
            Ok(())
        } else {
            Err(Self::expected_message_type(expected))
        }
    }
}

impl ReportableError for SMError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        match &self.kind {
            SMErrorKind::MakeEndpoint(e) => Some(e),
            SMErrorKind::Database(e) => Some(e),
            _ => None,
        }
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        self.backtrace.as_ref()
    }

    fn is_sentry_event(&self) -> bool {
        self.kind.is_sentry_event()
    }

    fn metric_label(&self) -> Option<&'static str> {
        match &self.kind {
            SMErrorKind::Database(e) => e.metric_label(),
            SMErrorKind::MakeEndpoint(e) => e.metric_label(),
            _ => None,
        }
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
    MakeEndpoint(#[source] ApcError),

    #[error("Client sent too many pings too often")]
    ExcessivePing,
}

impl SMErrorKind {
    /// Whether this error is reported to Sentry
    fn is_sentry_event(&self) -> bool {
        match self {
            SMErrorKind::Database(e) => e.is_sentry_event(),
            SMErrorKind::MakeEndpoint(e) => e.is_sentry_event(),
            SMErrorKind::Reqwest(_) | SMErrorKind::Internal(_) => true,
            _ => false,
        }
    }

    /// Whether this variant has a `Backtrace` captured
    ///
    /// Some Error variants have obvious call sites or more relevant backtraces
    /// in their sources and thus don't need a `Backtrace`. Furthermore
    /// backtraces are only captured for variants returning true from
    /// [Self::is_sentry_event].
    fn capture_backtrace(&self) -> bool {
        !matches!(self, SMErrorKind::MakeEndpoint(_))
    }
}

#[cfg(debug_assertions)]
/// Return a [SMErrorKind::Reqwest] [SMError] for tests
pub async fn __test_sm_reqwest_error() -> SMError {
    // An easily constructed reqwest::Error
    let e = reqwest::Client::builder()
        .https_only(true)
        .build()
        .unwrap()
        .get("http://example.com")
        .send()
        .await
        .unwrap_err();
    SMErrorKind::Reqwest(e).into()
}
