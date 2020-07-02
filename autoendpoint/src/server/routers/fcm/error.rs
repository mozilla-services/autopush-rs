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

    #[error("No registration token found for user")]
    NoRegistrationToken,

    #[error("No app ID found for user")]
    NoAppId,

    #[error("User has invalid app ID")]
    InvalidAppId,

    #[error(
        "This message is intended for a constrained device and is limited in \
         size. Converted buffer is too long by {0} bytes"
    )]
    TooMuchData(usize),
}

impl FcmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            FcmError::NoRegistrationToken | FcmError::NoAppId | FcmError::InvalidAppId => {
                StatusCode::GONE
            }
            FcmError::TooMuchData(_) => StatusCode::PAYLOAD_TOO_LARGE,
            FcmError::CredentialDecode(_) | FcmError::OAuthClientBuild(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            FcmError::TooMuchData(_) => Some(104),
            FcmError::NoRegistrationToken | FcmError::NoAppId | FcmError::InvalidAppId => Some(106),
            FcmError::CredentialDecode(_) | FcmError::OAuthClientBuild(_) => None,
        }
    }
}

impl From<FcmError> for ApiErrorKind {
    fn from(e: FcmError) -> Self {
        ApiErrorKind::Router(RouterError::Fcm(e))
    }
}
