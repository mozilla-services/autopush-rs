//! Error types and macros.
// TODO: Currently `Validation(#[cause] ValidationError)` may trigger some
// performance issues. The suggested fix is to Box ValidationError, however
// this cascades into Failure requiring std::error::Error being implemented
// which is out of scope.
#![allow(clippy::single_match, clippy::large_enum_variant)]

use std::convert::From;
use std::fmt;

use actix_web::{
    dev::{HttpResponseBuilder, ServiceResponse},
    error::ResponseError,
    http::StatusCode,
    middleware::errhandlers::ErrorHandlerResponse,
    HttpResponse, Result,
};
use failure::{Backtrace, Context, Fail};
use serde::{
    ser::{SerializeMap, SerializeSeq, Serializer},
    Serialize,
};

/// Common `Result` type.
pub type ApiResult<T> = Result<T, ApiError>;

/// How long the client should wait before retrying a conflicting write.
pub const RETRY_AFTER: u8 = 10;

/// Top-level error type.
#[derive(Debug)]
pub struct ApiError {
    inner: Context<ApiErrorKind>,
    status: StatusCode,
}

/// Top-level ErrorKind.
#[derive(Debug, Fail)]
pub enum ApiErrorKind {
    #[fail(display = "No app_data ServerState")]
    NoServerState,

    #[fail(display = "{}", _0)]
    Internal(String),
}

impl ApiError {
    pub fn kind(&self) -> &ApiErrorKind {
        self.inner.get_context()
    }

    pub fn render_404<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        // Replace the outbound error message with our own.
        let resp = HttpResponseBuilder::new(StatusCode::NOT_FOUND).finish();
        Ok(ErrorHandlerResponse::Response(ServiceResponse::new(
            res.request().clone(),
            resp.into_body(),
        )))
    }
}

impl From<actix_web::error::BlockingError<ApiError>> for ApiError {
    fn from(inner: actix_web::error::BlockingError<ApiError>) -> Self {
        match inner {
            actix_web::error::BlockingError::Error(e) => e,
            actix_web::error::BlockingError::Canceled => {
                ApiErrorKind::Internal("Db threadpool operation canceled".to_owned()).into()
            }
        }
    }
}

impl From<ApiError> for HttpResponse {
    fn from(inner: ApiError) -> Self {
        ResponseError::error_response(&inner)
    }
}

impl From<cadence::MetricError> for ApiError {
    fn from(inner: cadence::MetricError) -> Self {
        ApiErrorKind::Internal(inner.to_string()).into()
    }
}

impl From<std::io::Error> for ApiError {
    fn from(inner: std::io::Error) -> Self {
        ApiErrorKind::Internal(inner.to_string()).into()
    }
}

impl From<Context<ApiErrorKind>> for ApiError {
    fn from(inner: Context<ApiErrorKind>) -> Self {
        let status = match inner.get_context() {
            ApiErrorKind::NoServerState | ApiErrorKind::Internal(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        };

        Self { inner, status }
    }
}

impl ResponseError for ApiError {
    fn error_response(&self) -> HttpResponse {
        // To return a descriptive error response, this would work. We do not
        // unfortunately do that so that we can retain Sync 1.1 backwards compatibility
        // as the Python one does.
        // HttpResponse::build(self.status).json(self)
        //
        // So instead we translate our error to a backwards compatible one
        HttpResponse::build(self.status)
            .header("Retry-After", RETRY_AFTER.to_string())
            .finish()
    }
}

impl Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let size = if self.status == StatusCode::UNAUTHORIZED {
            2
        } else {
            3
        };

        let mut map = serializer.serialize_map(Some(size))?;
        map.serialize_entry("status", &self.status.as_u16())?;
        map.serialize_entry("reason", self.status.canonical_reason().unwrap_or(""))?;

        if self.status != StatusCode::UNAUTHORIZED {
            map.serialize_entry("errors", &self.inner.get_context())?;
        }

        map.end()
    }
}

impl Serialize for ApiErrorKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match *self {
            ApiErrorKind::Internal(ref description) => {
                serialize_string_to_array(serializer, description)
            }
            ApiErrorKind::NoServerState => {
                Serialize::serialize("No State information found", serializer)
            }
        }
    }
}

fn serialize_string_to_array<S, V>(serializer: S, value: V) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    V: fmt::Display,
{
    let mut seq = serializer.serialize_seq(Some(1))?;
    seq.serialize_element(&value.to_string())?;
    seq.end()
}

macro_rules! failure_boilerplate {
    ($error:ty, $kind:ty) => {
        impl Fail for $error {
            fn cause(&self) -> Option<&dyn Fail> {
                self.inner.cause()
            }

            fn backtrace(&self) -> Option<&Backtrace> {
                self.inner.backtrace()
            }
        }

        impl fmt::Display for $error {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.inner, formatter)
            }
        }

        impl From<$kind> for $error {
            fn from(kind: $kind) -> Self {
                Context::new(kind).into()
            }
        }
    };
}

failure_boilerplate!(ApiError, ApiErrorKind);

/*
macro_rules! from_error {
    ($from:ty, $to:ty, $to_kind:expr) => {
        impl From<$from> for $to {
            fn from(inner: $from) -> $to {
                $to_kind(inner).into()
            }
        }
    };
}
*/
