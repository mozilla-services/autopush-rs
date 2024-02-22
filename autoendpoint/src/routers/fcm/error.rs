use crate::error::ApiErrorKind;
use crate::routers::RouterError;

use actix_web::http::StatusCode;
use autopush_common::errors::ReportableError;
use backtrace::Backtrace;

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

    #[error("Invalid JSON response from FCM")]
    InvalidResponse(#[source] serde_json::Error, String, StatusCode),

    #[error("Empty response from FCM")]
    EmptyResponse(StatusCode),

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

            FcmError::DeserializeResponse(_)
            | FcmError::EmptyResponse(_)
            | FcmError::InvalidResponse(_, _, _) => StatusCode::BAD_GATEWAY,
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
            | FcmError::EmptyResponse(_)
            | FcmError::InvalidResponse(_, _, _)
            | FcmError::NoOAuthToken => None,
        }
    }
}

impl From<FcmError> for ApiErrorKind {
    fn from(e: FcmError) -> Self {
        ApiErrorKind::Router(RouterError::Fcm(e))
    }
}

impl ReportableError for FcmError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        None
    }

    fn backtrace(&self) -> Option<&Backtrace> {
        None
    }

    fn is_sentry_event(&self) -> bool {
        matches!(
            &self,
            FcmError::InvalidAppId(_)
                | FcmError::NoAppId
                | FcmError::EmptyResponse(_)
                | FcmError::InvalidResponse(_, _, _)
        )
    }

    fn metric_label(&self) -> Option<&'static str> {
        match &self {
            FcmError::InvalidAppId(_) | FcmError::NoAppId => {
                Some("notification.bridge.error.fcm.badappid")
            }
            _ => None,
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match self {
            FcmError::InvalidAppId(appid) => {
                vec![("app_id", appid.to_string())]
            }
            FcmError::EmptyResponse(status) => {
                vec![("status", status.to_string())]
            }
            FcmError::InvalidResponse(_, body, status) => {
                vec![("status", status.to_string()), ("body", body.to_owned())]
            }
            _ => vec![],
        }
    }
}
