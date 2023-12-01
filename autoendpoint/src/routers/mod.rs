//! Routers route notifications to user agents

use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::adm::error::AdmError;
use crate::routers::apns::error::ApnsError;
use crate::routers::fcm::error::FcmError;

use autopush_common::db::error::DbError;

use actix_web::http::StatusCode;
use actix_web::HttpResponse;
use async_trait::async_trait;
use std::collections::HashMap;
use thiserror::Error;

use self::stub::error::StubError;

pub mod adm;
pub mod apns;
mod common;
pub mod fcm;
pub mod stub;
pub mod webpush;

#[async_trait(?Send)]
pub trait Router {
    /// Validate that the user can use this router, and return data to be stored in
    /// the user's `router_data` field.
    fn register(
        &self,
        router_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, serde_json::Value>, RouterError>;

    /// Route a notification to the user
    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse>;
}

/// The response returned when a router routes a notification
#[derive(Debug, Eq, PartialEq)]
pub struct RouterResponse {
    pub status: StatusCode,
    pub headers: HashMap<&'static str, String>,
    pub body: Option<String>,
}

impl RouterResponse {
    /// Build a successful (200 OK) router response
    pub fn success(location: String, ttl: usize) -> Self {
        RouterResponse {
            status: StatusCode::OK,
            headers: {
                let mut map = HashMap::new();
                map.insert("Location", location);
                map.insert("TTL", ttl.to_string());
                map
            },
            body: None,
        }
    }
}

impl From<RouterResponse> for HttpResponse {
    fn from(router_response: RouterResponse) -> Self {
        let mut builder = HttpResponse::build(router_response.status);

        for (key, value) in router_response.headers {
            builder.insert_header((key, value));
        }

        builder.body(router_response.body.unwrap_or_default())
    }
}

#[derive(Debug, Error)]
pub enum RouterError {
    #[error(transparent)]
    Adm(#[from] AdmError),

    #[error(transparent)]
    Apns(#[from] ApnsError),

    #[error(transparent)]
    Fcm(#[from] FcmError),

    #[error(transparent)]
    Stub(#[from] StubError),

    #[error("Database error while saving notification")]
    SaveDb(#[source] DbError),

    #[error("User was deleted during routing")]
    UserWasDeleted,

    #[error(
        "This message is intended for a constrained device and is limited in \
         size. Converted buffer is too long by {0} bytes"
    )]
    TooMuchData(usize),

    #[error("Bridge authentication error")]
    Authentication,

    #[error("GCM Bridge authentication error")]
    GCMAuthentication,

    #[error("Bridge request timeout")]
    RequestTimeout,

    #[error("Error while connecting to bridge service")]
    Connect(#[source] reqwest::Error),

    #[error("Bridge reports user was not found")]
    NotFound,

    #[error("Bridge error, {status}: {message}")]
    Upstream { status: String, message: String },
}

impl RouterError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            RouterError::Adm(e) => e.status(),
            RouterError::Apns(e) => e.status(),
            RouterError::Fcm(e) => e.status(),
            RouterError::Stub(e) => e.status(),

            RouterError::SaveDb(_) => StatusCode::SERVICE_UNAVAILABLE,

            RouterError::UserWasDeleted | RouterError::NotFound => StatusCode::GONE,

            RouterError::TooMuchData(_) => StatusCode::PAYLOAD_TOO_LARGE,

            RouterError::Authentication
            | RouterError::GCMAuthentication
            | RouterError::RequestTimeout
            | RouterError::Connect(_)
            | RouterError::Upstream { .. } => StatusCode::BAD_GATEWAY,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            RouterError::Adm(e) => e.errno(),
            RouterError::Apns(e) => e.errno(),
            RouterError::Fcm(e) => e.errno(),
            RouterError::Stub(e) => e.errno(),

            RouterError::TooMuchData(_) => Some(104),

            RouterError::UserWasDeleted => Some(105),

            RouterError::NotFound => Some(106),

            RouterError::SaveDb(_) => Some(201),

            RouterError::Authentication => Some(901),

            RouterError::Connect(_) => Some(902),

            RouterError::RequestTimeout => Some(903),

            RouterError::GCMAuthentication => Some(904),

            RouterError::Upstream { .. } => None,
        }
    }

    pub fn metric_label(&self) -> Option<&'static str> {
        // NOTE: Some metrics are emitted for other Errors via handle_error
        // callbacks, whereas some are emitted via this method. These 2 should
        // be consoliated: https://mozilla-hub.atlassian.net/browse/SYNC-3695
        let err = match self {
            RouterError::Adm(AdmError::InvalidProfile | AdmError::NoProfile) => {
                "notification.bridge.error.adm.profile"
            }
            RouterError::Apns(ApnsError::SizeLimit(_)) => {
                "notification.bridge.error.apns.oversized"
            }
            RouterError::Fcm(FcmError::InvalidAppId(_) | FcmError::NoAppId) => {
                "notification.bridge.error.fcm.badappid"
            }
            RouterError::TooMuchData(_) => "notification.bridge.error.too_much_data",
            _ => "",
        };
        if !err.is_empty() {
            return Some(err);
        }
        None
    }

    pub fn is_sentry_event(&self) -> bool {
        match self {
            RouterError::Adm(e) => !matches!(e, AdmError::InvalidProfile | AdmError::NoProfile),
            // apns handle_error emits a metric for ApnsError::Unregistered
            RouterError::Apns(ApnsError::SizeLimit(_))
            | RouterError::Apns(ApnsError::Unregistered) => false,
            RouterError::Fcm(e) => !matches!(e, FcmError::InvalidAppId(_) | FcmError::NoAppId),
            // common handle_error emits metrics for these
            RouterError::Authentication
            | RouterError::GCMAuthentication
            | RouterError::Connect(_)
            | RouterError::NotFound
            | RouterError::RequestTimeout
            | RouterError::TooMuchData(_)
            | RouterError::Upstream { .. } => false,
            _ => true,
        }
    }

    pub fn extras(&self) -> Vec<(&str, String)> {
        match self {
            RouterError::Fcm(e) => e.extras(),
            _ => vec![],
        }
    }
}
