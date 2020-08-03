use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;

/// Errors that may occur in the Amazon Device Messaging router
#[derive(thiserror::Error, Debug)]
pub enum AdmError {
    #[error("Failed to decode the profile settings")]
    ProfileSettingsDecode(#[from] serde_json::Error),
}

impl AdmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            AdmError::ProfileSettingsDecode(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            AdmError::ProfileSettingsDecode(_) => None,
        }
    }
}

impl From<AdmError> for ApiErrorKind {
    fn from(e: AdmError) -> Self {
        ApiErrorKind::Router(RouterError::Adm(e))
    }
}
