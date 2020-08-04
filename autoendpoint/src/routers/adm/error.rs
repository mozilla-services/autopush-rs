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

    #[error("Error while connecting to ADM")]
    Connect(#[source] reqwest::Error),

    #[error("Unable to deserialize ADM response")]
    DeserializeResponse(#[source] reqwest::Error),

    #[error("ADM authentication error")]
    Authentication,

    #[error("ADM recipient no longer available")]
    NotFound,

    #[error("ADM error, {status}: {reason}")]
    Upstream { status: StatusCode, reason: String },

    #[error("ADM request timed out")]
    RequestTimeout,
}

impl AdmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            AdmError::ParseUrl(_) => StatusCode::BAD_REQUEST,

            AdmError::NotFound => StatusCode::GONE,

            AdmError::Http(_) | AdmError::ProfileSettingsDecode(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }

            AdmError::Connect(_)
            | AdmError::DeserializeResponse(_)
            | AdmError::Authentication
            | AdmError::Upstream { .. }
            | AdmError::RequestTimeout => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            AdmError::NotFound => Some(106),

            AdmError::Authentication => Some(901),

            AdmError::Connect(_) => Some(902),

            AdmError::RequestTimeout => Some(903),

            AdmError::Http(_)
            | AdmError::ParseUrl(_)
            | AdmError::ProfileSettingsDecode(_)
            | AdmError::DeserializeResponse(_)
            | AdmError::Upstream { .. } => None,
        }
    }
}

impl From<AdmError> for ApiErrorKind {
    fn from(e: AdmError) -> Self {
        ApiErrorKind::Router(RouterError::Adm(e))
    }
}
