use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;

use autopush_common::errors::ReportableError;

/// Errors that may occur in the Firebase Cloud Messaging router
#[derive(thiserror::Error, Clone, Debug)]
pub enum WnsError {
    #[error("Failed to decode the credential settings")]
    Credential,

    #[error("Missing registration token")]
    NoRegistrationToken,

    #[error("Missing Router Data")]
    NoRouterData,

    #[error("Missing app_id")]
    NoAppId,

    #[error("Invalid app_id provided")]
    InvalidAppId(String),

    #[error("General error")]
    General(String),
}

impl WnsError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            WnsError::Credential | WnsError::General(_) => StatusCode::INTERNAL_SERVER_ERROR,
            WnsError::NoRegistrationToken => StatusCode::BAD_REQUEST,
            WnsError::NoRouterData => StatusCode::INTERNAL_SERVER_ERROR,
            WnsError::NoAppId => StatusCode::INTERNAL_SERVER_ERROR,
            WnsError::InvalidAppId(_) => StatusCode::BAD_REQUEST,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            WnsError::NoRegistrationToken => Some(106),
            WnsError::InvalidAppId(_) => Some(107),
            _ => None,
        }
    }

    pub fn extras(&self) -> Vec<(&str, String)> {
        match self {
            WnsError::General(status) => {
                vec![("status", status.to_string())]
            }
            _ => vec![],
        }
    }
}

impl From<WnsError> for ApiErrorKind {
    fn from(e: WnsError) -> Self {
        ApiErrorKind::Router(RouterError::Wns(e))
    }
}

impl ReportableError for WnsError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        None
    }

    fn extras(&self) -> Vec<(&str, String)> {
        self.extras()
    }
}
