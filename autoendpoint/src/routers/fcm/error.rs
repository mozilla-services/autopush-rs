use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;

/// Errors that may occur in the Firebase Cloud Messaging router
#[derive(thiserror::Error, Debug)]
pub enum FcmError {
    #[error("Failed to decode the credential settings")]
    CredentialDecode(#[from] serde_json::Error),

    #[error("Error while building the OAuth client")]
    OAuthClientBuild(#[source] std::io::Error),

    #[error("Error while retrieving an OAuth token")]
    OAuthToken(#[from] yup_oauth2::Error),

    #[error("Unable to deserialize FCM response")]
    DeserializeResponse(#[source] reqwest::Error),

    #[error("No OAuth token was present")]
    NoOAuthToken,

    #[error("No registration token found for user")]
    NoRegistrationToken,

    #[error("No app ID found for user")]
    NoAppId,

    #[error("User has invalid app ID {0}")]
    InvalidAppId(String),
}

impl FcmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            FcmError::NoRegistrationToken | FcmError::NoAppId | FcmError::InvalidAppId(_) => {
                StatusCode::GONE
            }

            FcmError::CredentialDecode(_)
            | FcmError::OAuthClientBuild(_)
            | FcmError::OAuthToken(_)
            | FcmError::NoOAuthToken => StatusCode::INTERNAL_SERVER_ERROR,

            FcmError::DeserializeResponse(_) => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            FcmError::NoRegistrationToken | FcmError::NoAppId | FcmError::InvalidAppId(_) => {
                Some(106)
            }

            FcmError::CredentialDecode(_)
            | FcmError::OAuthClientBuild(_)
            | FcmError::OAuthToken(_)
            | FcmError::DeserializeResponse(_)
            | FcmError::NoOAuthToken => None,
        }
    }
}

impl From<FcmError> for ApiErrorKind {
    fn from(e: FcmError) -> Self {
        ApiErrorKind::Router(RouterError::Fcm(e))
    }
}
