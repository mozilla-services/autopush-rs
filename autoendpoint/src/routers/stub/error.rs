use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;

/// Errors that may occur in the Firebase Cloud Messaging router
#[derive(thiserror::Error, Clone, Debug)]
pub enum StubError {
    #[error("Failed to decode the credential settings")]
    Credential,

    #[error("Invalid JSON response from Test")]
    InvalidResponse,

    #[error("General error")]
    General(String),

    #[error("Missing User error")]
    Missing,
}

impl StubError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            StubError::Missing => StatusCode::GONE,

            StubError::Credential | StubError::General(_) => StatusCode::INTERNAL_SERVER_ERROR,

            StubError::InvalidResponse => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            StubError::Missing => Some(106),

            _ => None,
        }
    }

    pub fn extras(&self) -> Vec<(&str, String)> {
        match self {
            StubError::General(status) => {
                vec![("status", status.to_string())]
            }
            _ => vec![],
        }
    }
}

impl From<StubError> for ApiErrorKind {
    fn from(e: StubError) -> Self {
        ApiErrorKind::Router(RouterError::Stub(e))
    }
}
