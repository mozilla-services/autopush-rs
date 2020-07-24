use crate::error::ApiErrorKind;
use crate::routers::RouterError;
use actix_web::http::StatusCode;
use std::io;

/// Errors that may occur in the Apple Push Notification Service router
#[derive(thiserror::Error, Debug)]
pub enum ApnsError {
    #[error("Failed to decode the channel settings")]
    ChannelSettingsDecode(#[from] serde_json::Error),

    #[error("IO Error: {0}")]
    Io(#[from] io::Error),

    #[error("Error while setting up APNS clients: {0}")]
    ApnsClient(#[source] a2::Error),

    #[error("APNS error, {0}")]
    ApnsUpstream(#[source] a2::Error),

    #[error("No device token found for user")]
    NoDeviceToken,

    #[error("No release channel found for user")]
    NoReleaseChannel,

    #[error("Release channel is invalid")]
    InvalidReleaseChannel,

    #[error("Invalid APS data")]
    InvalidApsData,

    #[error("APNS recipient no longer available")]
    Unregistered,
}

impl ApnsError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            ApnsError::InvalidReleaseChannel | ApnsError::InvalidApsData => StatusCode::BAD_REQUEST,

            ApnsError::NoDeviceToken | ApnsError::NoReleaseChannel | ApnsError::Unregistered => {
                StatusCode::GONE
            }

            ApnsError::ChannelSettingsDecode(_) | ApnsError::Io(_) | ApnsError::ApnsClient(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }

            ApnsError::ApnsUpstream(_) => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            ApnsError::NoDeviceToken | ApnsError::NoReleaseChannel | ApnsError::Unregistered => {
                Some(106)
            }

            ApnsError::ChannelSettingsDecode(_)
            | ApnsError::Io(_)
            | ApnsError::ApnsClient(_)
            | ApnsError::ApnsUpstream(_)
            | ApnsError::InvalidReleaseChannel
            | ApnsError::InvalidApsData => None,
        }
    }
}

impl From<ApnsError> for ApiErrorKind {
    fn from(e: ApnsError) -> Self {
        ApiErrorKind::Router(RouterError::Apns(e))
    }
}
