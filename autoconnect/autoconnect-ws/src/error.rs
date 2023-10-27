use std::fmt;

use actix_ws::CloseCode;
use backtrace::Backtrace;

use autoconnect_ws_sm::{SMError, WebPushClient};
use autopush_common::{errors::ReportableError, sentry::event_from_error};

/// WebPush WebSocket Handler Errors
#[derive(Debug, thiserror::Error)]
pub struct WSError {
    pub kind: WSErrorKind,
    backtrace: Option<Backtrace>,
}

impl fmt::Display for WSError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

// Forward From impls to WSError from WSErrorKind. Because From is reflexive,
// this impl also takes care of From<WSErrorKind>.
impl<T> From<T> for WSError
where
    WSErrorKind: From<T>,
{
    fn from(item: T) -> Self {
        let kind = WSErrorKind::from(item);
        let backtrace = (kind.is_sentry_event() && kind.capture_backtrace()).then(Backtrace::new);
        Self { kind, backtrace }
    }
}

impl WSError {
    /// Return an `actix_ws::CloseCode` for the WS session Close frame
    pub fn close_code(&self) -> actix_ws::CloseCode {
        match &self.kind {
            WSErrorKind::SM(e) => e.close_code(),
            // TODO: applicable here?
            //WSErrorKind::Protocol(_) => CloseCode::Protocol,
            WSErrorKind::UnsupportedMessage(_) => CloseCode::Unsupported,
            _ => CloseCode::Error,
        }
    }

    /// Return a description for the WS session Close frame.
    ///
    /// Control frames are limited to 125 bytes so returns just the enum
    /// variant's name (via `strum::AsRefStr`)
    pub fn close_description(&self) -> &str {
        self.kind.as_ref()
    }

    /// Emit an event for this Error to Sentry
    pub fn capture_sentry_event(&self, client: Option<WebPushClient>) {
        if !self.is_sentry_event() {
            return;
        }
        let mut event = event_from_error(self);
        if let Some(client) = client {
            client.add_sentry_info(&mut event);
        }
        sentry::capture_event(event);
    }
}

impl ReportableError for WSError {
    fn backtrace(&self) -> Option<&Backtrace> {
        self.backtrace.as_ref()
    }

    fn is_sentry_event(&self) -> bool {
        self.kind.is_sentry_event()
    }

    fn metric_label(&self) -> Option<&'static str> {
        match &self.kind {
            WSErrorKind::SM(e) => e.metric_label(),
            // Legacy autoconnect ignored these: possibly not worth tracking
            WSErrorKind::Protocol(_) => Some("ua.ws_protocol_error"),
            _ => None,
        }
    }
}

#[derive(Debug, strum::AsRefStr, thiserror::Error)]
pub enum WSErrorKind {
    #[error("State error: {0}")]
    SM(#[from] SMError),

    #[error("Couldn't parse WebSocket message JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("WebSocket protocol error: {0}")]
    Protocol(#[from] actix_ws::ProtocolError),

    #[error("WebSocket session unexpectedly closed: {0}")]
    SessionClosed(#[from] actix_ws::Closed),

    #[error("Unsupported WebSocket message: {0}")]
    UnsupportedMessage(String),

    #[error("WebSocket stream unexpectedly closed")]
    StreamClosed,

    #[error("Timeout waiting for handshake")]
    HandshakeTimeout,

    #[error("Timeout waiting for Pong")]
    PongTimeout,

    #[error("ClientRegistry unexpectedly disconnected")]
    RegistryDisconnected,
}

impl WSErrorKind {
    /// Whether this error is reported to Sentry
    fn is_sentry_event(&self) -> bool {
        match self {
            WSErrorKind::SM(e) => e.is_sentry_event(),
            WSErrorKind::RegistryDisconnected => true,
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
        // Nothing currently (RegistryDisconnected has a unique call site) but
        // we may want to capture other variants in the future
        false
    }
}
