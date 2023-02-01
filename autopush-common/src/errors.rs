//! Error handling for Rust
//!

use std::any::Any;
use backtrace::Backtrace; // Sentry 0.29 uses the backtrace crate, not std::backtrace
use std::fmt::{self, Display};
use std::io;
use std::num;

use actix_web::{
    dev::ServiceResponse, http::StatusCode, middleware::ErrorHandlerResponse, HttpResponseBuilder,
};


use thiserror::Error;

/// Render a 404 response
pub fn render_404<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
    // Replace the outbound error message with our own.
    let resp = HttpResponseBuilder::new(StatusCode::NOT_FOUND).finish();
    Ok(ErrorHandlerResponse::Response(
        res.into_response(resp).map_into_right_body(),
    ))
}

/// AutoPush Common error (To distinguish from endpoint's ApiError)
#[derive(Debug)]
pub struct ApcError {
    pub kind: ApcErrorKind,
    pub backtrace: Box<Backtrace>,
}

// Print out the error and backtrace, including source errors
impl Display for ApcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error: {}\nBacktrace: \n{:?}", self.kind, self.backtrace)?;

        // Go down the chain of errors
        let mut error: &dyn std::error::Error = &self.kind;
        while let Some(source) = error.source() {
            write!(f, "\n\nCaused by: {source}")?;
            error = source;
        }

        Ok(())
    }
}

impl std::error::Error for ApcError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.kind.source()
    }
}

// Forward From impls to ApiError from ApiErrorKind. Because From is reflexive,
// this impl also takes care of From<ApiErrorKind>.
impl<T> From<T> for ApcError
where
    ApcErrorKind: From<T>,
{
    fn from(item: T) -> Self {
        ApcError {
            kind: ApcErrorKind::from(item),
            backtrace: Box::new(Backtrace::new()),  // or std::backtrace::Backtrace::capture()
        }
    }
}

#[derive(Error, Debug)]
pub enum ApcErrorKind {
    #[error(transparent)]
    Ws(#[from] tungstenite::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Httparse(#[from] httparse::Error),
    #[error(transparent)]
    MetricError(#[from] cadence::MetricError),
    #[error(transparent)]
    UuidError(#[from] uuid::Error),
    #[error(transparent)]
    ParseIntError(#[from] num::ParseIntError),
    #[error(transparent)]
    ParseUrlError(#[from] url::ParseError),
    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),
    #[error(transparent)]
    DbError(#[from] crate::db::error::DbError),
    #[error("Error while validating token")]
    TokenHashValidation(#[source] openssl::error::ErrorStack),
    #[error("Error while creating secret")]
    RegistrationSecretHash(#[source] openssl::error::ErrorStack),

    #[error("thread panicked")]
    Thread(Box<dyn Any + Send>),
    #[error("websocket pong timeout")]
    PongTimeout,
    #[error("repeat uaid disconnect")]
    RepeatUaidDisconnect,
    #[error("invalid state transition, from: {0}, to: {1}")]
    InvalidStateTransition(String, String),
    #[error("invalid json: {0}")]
    InvalidClientMessage(String),
    #[error("server error fetching messages")]
    MessageFetch,
    #[error("unable to send to client")]
    SendError,
    #[error("client sent too many pings")]
    ExcessivePing,

    #[error("Broadcast Error: {0}")]
    BroadcastError(String),
    #[error("Payload Error: {0}")]
    PayloadError(String),
    #[error("General Error: {0}")]
    GeneralError(String),
    #[error("Database Error: {0}")]
    DatabaseError(String),

    #[error("Endpoint Error: [{0}] {1}")]
    EndpointError(&'static str, String),

    // TODO: option this.
    #[error("Rusoto Error: {0}")]
    RusotoError(String),
}

pub type Result<T> = std::result::Result<T, ApcError>;

// pub type MyFuture<T> = Box<dyn Future<Item = T, Error = ApcError>>;
