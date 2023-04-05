use actix_ws::CloseCode;

use autoconnect_ws_sm::SMError;

/// WebPush WebSocket Handler Errors
#[derive(thiserror::Error, Debug)]
pub enum WSError {
    #[error("State machine error: {0}")]
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

    #[error("ClientRegistry disconnect unexpectedly failed (Client not connected)")]
    RegistryNotConnected,
}

impl WSError {
    pub fn close_code(&self) -> actix_ws::CloseCode {
        match self {
            WSError::SM(e) => e.close_code(),
            // TODO: applicable here?
            //WSError::Protocol(_) => CloseCode::Protocol,
            WSError::UnsupportedMessage(_) => CloseCode::Unsupported,
            _ => CloseCode::Error,
        }
    }
}
