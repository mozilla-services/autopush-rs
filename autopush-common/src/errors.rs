//! Error handling for Rust

use std::any::Any;
use std::error::Error;
use std::fmt::{self, Display};
use std::num;

use actix_web::{
    dev::ServiceResponse, http::StatusCode, middleware::ErrorHandlerResponse, HttpResponseBuilder,
    Result,
};

use backtrace::Backtrace;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use thiserror::Error;

pub type ApiResult<T> = Result<T, ApiError>;

/// Render a 404 response
pub fn render_404<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
    // Replace the outbound error message with our own.
    let resp = HttpResponseBuilder::new(StatusCode::NOT_FOUND).finish();
    Ok(ErrorHandlerResponse::Response(
        res.into_response(resp).map_into_right_body(),
    ))
}

/// The main error type.
#[derive(Debug)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub backtrace: Backtrace,
}

impl Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Error: {}\nBacktrace: \n{:?}", self.kind, self.backtrace)?;

        // Go down the chain of errors
        let mut error: &dyn Error = &self.kind;
        while let Some(source) = error.source() {
            write!(f, "\n\nCaused by: {}", source)?;
            error = source;
        }

        Ok(())
    }
}

/*
impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.kind.source()
    }
}
*/

// Forward From impls to ApiError from ApiErrorKind. Because From is reflexive,
// this impl also takes care of From<ApiErrorKind>.
impl<T> From<T> for ApiError
where
    ApiErrorKind: From<T>,
{
    fn from(item: T) -> Self {
        ApiError {
            kind: ApiErrorKind::from(item),
            backtrace: Backtrace::new(),
        }
    }
}

impl Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let status = self.kind.status();
        let mut map = serializer.serialize_map(Some(5))?;

        map.serialize_entry("code", &status.as_u16())?;
        map.serialize_entry("error", &status.canonical_reason())?;
        map.serialize_entry("message", &self.kind.to_string())?;
        // map.serialize_entry("more_info", ERROR_URL)?;
        map.end()
    }
}

impl From<&str> for ApiError {
    fn from(message: &str) -> Self {
        ApiErrorKind::GeneralError(message.to_owned()).into()
    }
}

/*
impl From<ApiErrorKind> for ApiError {
    fn from(err: ApiErrorKind) -> Self {
        Self{
            kind: err,
            backtrace: Backtrace::new()
        }
    }
}
*/

#[derive(Debug, Error)]
pub enum ApiErrorKind {
    #[error(transparent)]
    Ws(#[from] tungstenite::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    ErrorStack(#[from] openssl::error::ErrorStack),

    #[error(transparent)]
    DecodeError(#[from] base64::DecodeError),

    #[error(transparent)]
    Httparse(#[from] httparse::Error),

    #[error(transparent)]
    MetricError(#[from] cadence::MetricError),

    #[error(transparent)]
    UuidError(#[from] uuid::Error),

    #[error(transparent)]
    ParseIntError(#[from] num::ParseIntError),

    #[error(transparent)]
    ParseError(#[from] url::ParseError),

    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),

    #[error("thread panicked")]
    Thread(Box<dyn Any + Send>),

    #[error("websocket pong timeout")]
    PongTimeout(),

    #[error("repeated uaid disconnect")]
    RepeatUaidDisconnect(),

    #[error("pings are not far enough apart")]
    ExcessivePing(),

    #[error("invalid state transition, from: {0}, to: {1}")]
    InvalidStateTransition(String, String),

    #[error("Invalid json: {0}")]
    InvalidClientMessage(String),

    #[error("server error fetching messages")]
    MessageFetch(),

    #[error("unable to send to client")]
    SendError(),

    #[error("Database Error:")]
    DatabaseError(#[from] crate::db::error::DbError),

    // TODO: option this.
    #[error("Rusoto Error: {0}")]
    RusotoError(String),

    #[error("General Error: {0}")]
    GeneralError(String),

    #[error("Log Check: OK")]
    LogCheck(),
}

impl ApiErrorKind {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        trace!("Returning error: {}", self.metric_label());
        //TODO: Fill these in
        match self {
            Self::LogCheck() => StatusCode::IM_A_TEAPOT,
            _ => StatusCode::from_u16(500).unwrap(),
        }
    }

    pub fn metric_label(&self) -> &'static str {
        match self {
            Self::Ws(_) => "websocket_error",
            Self::Io(_) => "io_error",
            Self::Json(_) => "json_error",
            Self::Httparse(_) => "httparse_error",
            Self::MetricError(_) => "metric_error",
            Self::DecodeError(_) => "decode_error",
            Self::ErrorStack(_) => "error_stack",
            Self::UuidError(_) => "uuid_error",
            Self::ParseIntError(_) => "parse_int_error",
            Self::ParseError(_) => "parse_url_error",
            Self::ConfigError(_) => "config_error",
            Self::Thread(_) => "thread_error",
            Self::PongTimeout() => "pong_timeout",
            Self::RepeatUaidDisconnect() => "repeated_uaid_disconnect",
            Self::ExcessivePing() => "excessive_pings",
            Self::InvalidStateTransition(_, _) => "invalid_state_transition",
            Self::InvalidClientMessage(_) => "invalid_client_message",
            Self::MessageFetch() => "message_fetch",
            Self::SendError() => "send_error",
            Self::DatabaseError(_) => "database_error",
            Self::RusotoError(_) => "rusoto_error",
            Self::GeneralError(_) => "general_error",
            Self::LogCheck() => "log_check",
        }
    }
}
