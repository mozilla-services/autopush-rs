use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;

/// Errors that may occur in the Amazon Device Messaging router
#[derive(thiserror::Error, Debug)]
pub enum AdmError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Error while building a URL: {0}")]
    ParseUrl(#[from] url::ParseError),

    #[error("Failed to decode the profile settings")]
    ProfileSettingsDecode(#[from] serde_json::Error),

    #[error("Unable to deserialize ADM response")]
    DeserializeResponse(#[source] reqwest::Error),

    #[error("No registration ID found for user")]
    NoRegistrationId,

    #[error("No ADM profile found for user")]
    NoProfile,

    #[error("User has invalid ADM profile")]
    InvalidProfile,
}

impl AdmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            AdmError::ParseUrl(_) => StatusCode::BAD_REQUEST,

            AdmError::NoRegistrationId | AdmError::NoProfile | AdmError::InvalidProfile => {
                StatusCode::GONE
            }

            AdmError::Http(_) | AdmError::ProfileSettingsDecode(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }

            AdmError::DeserializeResponse(_) => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            AdmError::NoRegistrationId | AdmError::NoProfile | AdmError::InvalidProfile => {
                Some(106)
            }

            AdmError::Http(_)
            | AdmError::ParseUrl(_)
            | AdmError::ProfileSettingsDecode(_)
            | AdmError::DeserializeResponse(_) => None,
        }
    }
}

impl From<AdmError> for ApiErrorKind {
    fn from(e: AdmError) -> Self {
        ApiErrorKind::Router(RouterError::Adm(e))
    }
}
