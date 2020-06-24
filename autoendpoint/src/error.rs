//! Error types and transformations

use crate::server::VapidError;
use actix_web::{
    dev::{HttpResponseBuilder, ServiceResponse},
    error::{PayloadError, ResponseError},
    http::StatusCode,
    middleware::errhandlers::ErrorHandlerResponse,
    HttpResponse, Result,
};
use backtrace::Backtrace;
use serde::ser::SerializeMap;
use serde::{Serialize, Serializer};
use std::error::Error;
use std::fmt::{self, Display};
use thiserror::Error;

/// Common `Result` type.
pub type ApiResult<T> = Result<T, ApiError>;

/// How long the client should wait before retrying a conflicting write.
pub const RETRY_AFTER: u8 = 10;

/// The main error type.
#[derive(Debug)]
pub struct ApiError {
    pub kind: ApiErrorKind,
    pub backtrace: Backtrace,
}

impl ApiError {
    /// Render a 404 response
    pub fn render_404<B>(res: ServiceResponse<B>) -> Result<ErrorHandlerResponse<B>> {
        // Replace the outbound error message with our own.
        let resp = HttpResponseBuilder::new(StatusCode::NOT_FOUND).finish();
        Ok(ErrorHandlerResponse::Response(ServiceResponse::new(
            res.request().clone(),
            resp.into_body(),
        )))
    }
}

/// The possible errors this application could encounter
#[derive(Debug, Error)]
pub enum ApiErrorKind {
    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Metrics(#[from] cadence::MetricError),

    #[error(transparent)]
    Validation(#[from] validator::ValidationErrors),

    // PayloadError does not implement std Error
    #[error("{0}")]
    PayloadError(PayloadError),

    #[error(transparent)]
    VapidError(#[from] VapidError),

    #[error(transparent)]
    Uuid(#[from] uuid::Error),

    #[error(transparent)]
    Jwt(#[from] jsonwebtoken::errors::Error),

    #[error("Error while validating token")]
    TokenHashValidation(#[source] openssl::error::ErrorStack),

    #[error("Database error: {0}")]
    Database(#[source] autopush_common::errors::Error),

    #[error("Invalid token")]
    InvalidToken,

    #[error("No such subscription")]
    NoSubscription,

    /// A specific issue with the encryption headers
    #[error("{0}")]
    InvalidEncryption(String),

    #[error("Data payload must be smaller than {} bytes", .0)]
    PayloadTooLarge(usize),

    /// Used if the API version given is not v1 or v2
    #[error("Invalid API version")]
    InvalidApiVersion,

    #[error("User was deleted during routing")]
    UserWasDeleted,

    #[error("{0}")]
    Internal(String),
}

impl ApiErrorKind {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            ApiErrorKind::PayloadError(e) => e.status_code(),

            ApiErrorKind::Validation(_)
            | ApiErrorKind::InvalidEncryption(_)
            | ApiErrorKind::TokenHashValidation(_)
            | ApiErrorKind::Uuid(_) => StatusCode::BAD_REQUEST,

            ApiErrorKind::NoSubscription | ApiErrorKind::UserWasDeleted => StatusCode::GONE,

            ApiErrorKind::VapidError(_) | ApiErrorKind::Jwt(_) => StatusCode::UNAUTHORIZED,

            ApiErrorKind::InvalidToken | ApiErrorKind::InvalidApiVersion => StatusCode::NOT_FOUND,

            ApiErrorKind::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,

            ApiErrorKind::Io(_)
            | ApiErrorKind::Metrics(_)
            | ApiErrorKind::Database(_)
            | ApiErrorKind::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

// Print out the error and backtrace, including source errors
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

impl Error for ApiError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.kind.source()
    }
}

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

impl ResponseError for ApiError {
    fn error_response(&self) -> HttpResponse {
        // To return a descriptive error response, this would work. We do not
        // unfortunately do that so that we can retain Sync 1.1 backwards compatibility
        // as the Python one does.
        // HttpResponse::build(self.status).json(self)
        //
        // So instead we translate our error to a backwards compatible one
        HttpResponse::build(self.kind.status())
            .header("Retry-After", RETRY_AFTER.to_string())
            .finish()
    }
}

// TODO: Use the same schema as documented here?
//       https://autopush.readthedocs.io/en/latest/http.html#response
impl Serialize for ApiError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let status = self.kind.status();
        let size = if status == StatusCode::UNAUTHORIZED {
            2
        } else {
            3
        };

        let mut map = serializer.serialize_map(Some(size))?;
        map.serialize_entry("status", &status.as_u16())?;
        map.serialize_entry("reason", status.canonical_reason().unwrap_or(""))?;

        if status != StatusCode::UNAUTHORIZED {
            map.serialize_entry("errors", &self.kind.to_string())?;
        }

        map.end()
    }
}
