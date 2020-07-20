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

    #[error("Error while retrieving an OAuth token")]
    OAuthToken(#[from] yup_oauth2::Error),

    #[error("Error while connecting to FCM")]
    FcmConnect(#[source] reqwest::Error),

    #[error("Unable to deserialize FCM response")]
    DeserializeResponse(#[source] reqwest::Error),

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

    #[error("FCM authentication error")]
    FcmAuthentication,

    #[error("FCM recipient no longer available")]
    FcmNotFound,

    #[error("FCM request timed out")]
    FcmRequestTimeout,

    #[error("FCM error, {status}: {message}")]
    FcmUpstream { status: String, message: String },

    #[error("Unknown FCM error")]
    FcmUnknown,
}

impl FcmError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            FcmError::NoRegistrationToken
            | FcmError::NoAppId
            | FcmError::InvalidAppId
            | FcmError::FcmNotFound => StatusCode::GONE,

            FcmError::TooMuchData(_) => StatusCode::PAYLOAD_TOO_LARGE,

            FcmError::CredentialDecode(_)
            | FcmError::OAuthClientBuild(_)
            | FcmError::OAuthToken(_) => StatusCode::INTERNAL_SERVER_ERROR,

            FcmError::FcmConnect(_)
            | FcmError::DeserializeResponse(_)
            | FcmError::FcmAuthentication
            | FcmError::FcmRequestTimeout
            | FcmError::FcmUpstream { .. }
            | FcmError::FcmUnknown => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            FcmError::TooMuchData(_) => Some(104),

            FcmError::NoRegistrationToken
            | FcmError::NoAppId
            | FcmError::InvalidAppId
            | FcmError::FcmNotFound => Some(106),

            FcmError::FcmAuthentication => Some(901),

            FcmError::FcmConnect(_) => Some(902),

            FcmError::FcmRequestTimeout => Some(903),

            FcmError::CredentialDecode(_)
            | FcmError::OAuthClientBuild(_)
            | FcmError::OAuthToken(_)
            | FcmError::DeserializeResponse(_)
            | FcmError::FcmUpstream { .. }
            | FcmError::FcmUnknown => None,
        }
    }
}

impl From<FcmError> for ApiErrorKind {
    fn from(e: FcmError) -> Self {
        ApiErrorKind::Router(RouterError::Fcm(e))
    }
}
