use actix_ws::CloseCode;

use autopush_common::db::error::DbError;

/// WebSocket state machine errors
#[derive(thiserror::Error, Debug)]
pub enum SMError {
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

impl SMError {
    pub fn close_code(&self) -> actix_ws::CloseCode {
        match self {
            // TODO: applicable here?
            //SMError::InvalidMessage(_) => CloseCode::Invalid,
            SMError::UaidReset => CloseCode::Normal,
            _ => CloseCode::Error,
        }
    }
}
