use actix_ws::CloseCode;

use autoconnect_ws_sm::{SMError, WebPushClient};

// TODO: WSError should likely include a backtrace
/// WebPush WebSocket Handler Errors
#[derive(Debug, strum::AsRefStr, thiserror::Error)]
pub enum WSError {
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

    #[error("ClientRegistry unexpectedly disconnected")]
    RegistryDisconnected,
}

impl WSError {
    /// Return an `actix_ws::CloseCode` for the WS session Close frame
    pub fn close_code(&self) -> actix_ws::CloseCode {
        match self {
            WSError::SM(e) => e.close_code(),
            // TODO: applicable here?
            //WSError::Protocol(_) => CloseCode::Protocol,
            WSError::UnsupportedMessage(_) => CloseCode::Unsupported,
            _ => CloseCode::Error,
        }
    }

    /// Return a description for the WS session Close frame.
    ///
    /// Control frames are limited to 125 bytes so returns just the enum
    /// variant's name (via `strum::AsRefStr`)
    pub fn close_description(&self) -> &str {
        self.as_ref()
    }

    /// Whether this error is reported to sentry
    pub fn is_sentry_event(&self) -> bool {
        true
    }

    pub fn to_sentry_event(&self, client: &WebPushClient) -> sentry::protocol::Event<'static> {
        let mut event = sentry::event_from_error(self);
        // TODO:
        //event.exception.last_mut().unwrap().stacktrace =
        //    sentry::integrations::backtrace::backtrace_to_stacktrace(&self.backtrace);

        event.user = Some(sentry::User {
            id: Some(client.uaid.as_simple().to_string()),
            ..Default::default()
        });
        let ua_info = client.ua_info.clone();
        event
            .tags
            .insert("ua_name".to_owned(), ua_info.browser_name);
        event
            .tags
            .insert("ua_os_family".to_owned(), ua_info.metrics_os);
        event
            .tags
            .insert("ua_os_ver".to_owned(), ua_info.os_version);
        event
            .tags
            .insert("ua_browser_family".to_owned(), ua_info.metrics_browser);
        event
            .tags
            .insert("ua_browser_ver".to_owned(), ua_info.browser_version);
        event
    }
}
