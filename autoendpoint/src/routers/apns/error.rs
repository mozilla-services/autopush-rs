use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;

/// Errors that may occur in the Apple Push Notification Service router
#[derive(thiserror::Error, Debug)]
pub enum ApnsError {
    #[error("Failed to decode the channel settings")]
    ChannelSettingsDecode(#[from] serde_json::Error),

    #[error("Error while connecting to APNS: {0}")]
    ApnsClient(#[source] a2::Error),
}

impl ApnsError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            ApnsError::ChannelSettingsDecode(_) | ApnsError::ApnsClient(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            ApnsError::ChannelSettingsDecode(_) | ApnsError::ApnsClient(_) => None,
        }
    }
}

impl From<ApnsError> for ApiErrorKind {
    fn from(e: ApnsError) -> Self {
        ApiErrorKind::Router(RouterError::Apns(e))
    }
}
