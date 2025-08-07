//! Error handling for common autopush functions

use std::fmt::{self, Display};
use std::io;
use std::num;

use actix_web::{
    dev::ServiceResponse, http::StatusCode, middleware::ErrorHandlerResponse, HttpResponse,
    HttpResponseBuilder, ResponseError,
};
// Sentry 0.29 uses the backtrace crate, not std::backtrace
use backtrace::Backtrace;
use serde::ser::{Serialize, SerializeMap, Serializer};
use thiserror::Error;

#[cfg(feature = "reliable_report")]
use redis::RedisError;

pub type Result<T> = std::result::Result<T, ApcError>;

/// Render a 404 response
pub fn render_404<B>(
    res: ServiceResponse<B>,
) -> std::result::Result<ErrorHandlerResponse<B>, actix_web::Error> {
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

impl Display for ApcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.kind.fmt(f)
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
            backtrace: Box::new(Backtrace::new()), // or std::backtrace::Backtrace::capture()
        }
    }
}

/// Return a structured response error for the ApcError
impl ResponseError for ApcError {
    fn status_code(&self) -> StatusCode {
        self.kind.status()
    }

    fn error_response(&self) -> HttpResponse {
        let mut builder = HttpResponse::build(self.status_code());
        builder.json(self)
    }
}

impl Serialize for ApcError {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let status = self.kind.status();
        let mut map = serializer.serialize_map(Some(5))?;

        map.serialize_entry("code", &status.as_u16())?;
        map.serialize_entry("error", &status.canonical_reason())?;
        map.serialize_entry("message", &self.kind.to_string())?;
        // TODO: errno and url?
        map.end()
    }
}

#[derive(Error, Debug)]
pub enum ApcErrorKind {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    UuidError(#[from] uuid::Error),
    #[error(transparent)]
    ParseIntError(#[from] num::ParseIntError),
    #[error(transparent)]
    ParseUrlError(#[from] url::ParseError),
    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),
    #[cfg(feature = "reliable_report")]
    #[error(transparent)]
    RedisError(#[from] RedisError),
    #[error("Broadcast Error: {0}")]
    BroadcastError(String),
    #[error("Payload Error: {0}")]
    PayloadError(String),
    #[error("Invalid message ID")]
    InvalidMessageId,
    #[error("General Error: {0}")]
    GeneralError(String),
}

impl ApcErrorKind {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            Self::ParseIntError(_) | Self::ParseUrlError(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn is_sentry_event(&self) -> bool {
        match self {
            // TODO: Add additional messages to ignore here.
            // Non-actionable Endpoint errors
            Self::PayloadError(_) => false,
            _ => true,
        }
    }

    pub fn metric_label(&self) -> Option<&'static str> {
        // TODO: add labels for skipped stuff
        match self {
            Self::PayloadError(_) => Some("payload"),
            _ => None,
        }
    }
}

/// Interface for reporting our Error types to Sentry or as metrics
pub trait ReportableError: std::error::Error {
    /// Like [Error::source] but returns the source (if any) of this error as a
    /// [ReportableError] if it implements the trait. Otherwise callers of this
    /// method will likely subsequently call [Error::source] to return the
    /// source (if any) as the parent [Error] trait.
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        None
    }

    /// Return a `Backtrace` for this Error if one was captured
    fn backtrace(&self) -> Option<&Backtrace> {
        None
    }
    /// Whether this error is reported to Sentry
    fn is_sentry_event(&self) -> bool {
        true
    }

    /// Errors that don't emit Sentry events (!is_sentry_event()) emit an
    /// increment metric instead with this label
    fn metric_label(&self) -> Option<&'static str> {
        None
    }

    /// Experimental: return tag key value pairs for metrics and Sentry
    fn tags(&self) -> Vec<(&str, String)> {
        vec![]
    }

    /// Experimental: return key value pairs for Sentry Event's extra data
    /// TODO: should probably return Vec<(&str, Value)> or Vec<(String, Value)>
    fn extras(&self) -> Vec<(&str, String)> {
        vec![]
    }
}

impl ReportableError for ApcError {
    fn backtrace(&self) -> Option<&Backtrace> {
        Some(&self.backtrace)
    }

    fn is_sentry_event(&self) -> bool {
        self.kind.is_sentry_event()
    }

    fn metric_label(&self) -> Option<&'static str> {
        self.kind.metric_label()
    }
}
