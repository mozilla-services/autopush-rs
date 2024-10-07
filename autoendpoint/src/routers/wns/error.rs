use crate::error::ApiErrorKind;
use crate::routers::RouterError;

use autopush_common::errors::ReportableError;
use reqwest::StatusCode;

/// Errors that may occur in the Firebase Cloud Messaging router
#[derive(thiserror::Error, Debug)]
pub enum WnsError {
    #[error("Failed to decode the credential settings")]
    CredentialDecode(#[from] serde_json::Error),

    #[error("Error while building the OAuth client")]
    OAuthClientBuild(#[source] std::io::Error),

    #[error("Error while retrieving an OAuth token")]
    OAuthToken(#[from] yup_oauth2::Error),

    #[error("Unable to deserialize WNS response")]
    DeserializeResponse(#[source] reqwest::Error),

    #[error("Invalid JSON response from WNS")]
    InvalidResponse(#[source] serde_json::Error, String, StatusCode),

    #[error("Empty response from WNS")]
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

impl WnsError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            WnsError::NoRegistrationToken | WnsError::NoAppId | WnsError::InvalidAppId(_) => {
                StatusCode::GONE
            }

            WnsError::CredentialDecode(_)
            | WnsError::OAuthClientBuild(_)
            | WnsError::OAuthToken(_)
            | WnsError::NoOAuthToken => StatusCode::INTERNAL_SERVER_ERROR,

            WnsError::DeserializeResponse(_)
            | WnsError::EmptyResponse(_)
            | WnsError::InvalidResponse(_, _, _) => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            WnsError::NoRegistrationToken | WnsError::NoAppId | WnsError::InvalidAppId(_) => {
                Some(106)
            }

            WnsError::CredentialDecode(_)
            | WnsError::OAuthClientBuild(_)
            | WnsError::OAuthToken(_)
            | WnsError::DeserializeResponse(_)
            | WnsError::EmptyResponse(_)
            | WnsError::InvalidResponse(_, _, _)
            | WnsError::NoOAuthToken => None,
        }
    }
}

impl From<WnsError> for ApiErrorKind {
    fn from(e: WnsError) -> Self {
        ApiErrorKind::Router(RouterError::Wns(e))
    }
}

impl ReportableError for WnsError {
    fn is_sentry_event(&self) -> bool {
        matches!(&self, WnsError::InvalidAppId(_) | WnsError::NoAppId)
    }

    fn metric_label(&self) -> Option<&'static str> {
        match &self {
            WnsError::InvalidAppId(_) | WnsError::NoAppId => {
                Some("notification.bridge.error.wns.badappid")
            }
            _ => None,
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match self {
            WnsError::InvalidAppId(appid) => {
                vec![("app_id", appid.to_string())]
            }
            WnsError::EmptyResponse(status) => {
                vec![("status", status.to_string())]
            }
            WnsError::InvalidResponse(_, body, status) => {
                vec![("status", status.to_string()), ("body", body.to_owned())]
            }
            _ => vec![],
        }
    }
}
