use crate::error::ApiErrorKind;
use crate::server::RouterError;
use actix_web::http::StatusCode;

/// Errors that may occur in the Firebase Cloud Messaging router
#[derive(thiserror::Error, Debug)]
pub enum FcmError {
    #[error("Failed to decode the credential settings")]
    CredentialDecode(#[from] serde_json::Error),

    #[error("Error while building the OAuth client")]
    OAuthClientBuild(#[source] std::io::Error),
}

impl FcmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            FcmError::CredentialDecode(_) | FcmError::OAuthClientBuild(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            FcmError::CredentialDecode(_) | FcmError::OAuthClientBuild(_) => None,
        }
    }
}

impl From<FcmError> for ApiErrorKind {
    fn from(e: FcmError) -> Self {
        ApiErrorKind::Router(RouterError::Fcm(e))
    }
}
