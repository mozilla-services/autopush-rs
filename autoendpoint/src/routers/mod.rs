//! Routers route notifications to user agents

use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::apns::error::ApnsError;
use crate::routers::fcm::error::FcmError;

use autopush_common::db::error::DbError;

use actix_web::http::StatusCode;
use actix_web::HttpResponse;
use async_trait::async_trait;
use autopush_common::errors::ReportableError;
use std::collections::HashMap;
use thiserror::Error;

#[cfg(feature = "stub")]
use self::stub::error::StubError;
pub mod apns;
mod common;
pub mod fcm;
#[cfg(feature = "stub")]
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
    /// Build a successful (201 CREATED) router response
    /// (Note, we return a 201 here for [RFC](https://datatracker.ietf.org/doc/html/rfc8030#section-5) compliance)
    pub fn success(location: String, ttl: usize) -> Self {
        RouterResponse {
            status: StatusCode::CREATED,
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
    Apns(#[from] ApnsError),

    #[error(transparent)]
    Fcm(#[from] FcmError),

    #[cfg(feature = "stub")]
    #[error(transparent)]
    Stub(#[from] StubError),

    #[error("Database error while saving notification")]
    SaveDb(#[source] DbError, Option<String>),

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
            RouterError::Apns(e) => e.status(),
            RouterError::Fcm(e) => StatusCode::from_u16(e.status().as_u16()).unwrap_or_default(),

            RouterError::SaveDb(e, _) => e.status(),
            #[cfg(feature = "stub")]
            RouterError::Stub(e) => e.status(),

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
            RouterError::Apns(e) => e.errno(),
            RouterError::Fcm(e) => e.errno(),

            #[cfg(feature = "stub")]
            RouterError::Stub(e) => e.errno(),

            RouterError::TooMuchData(_) => Some(104),

            RouterError::UserWasDeleted => Some(105),

            RouterError::NotFound => Some(106),

            RouterError::SaveDb(_, _) => Some(201),

            RouterError::Authentication => Some(901),

            RouterError::Connect(_) => Some(902),

            RouterError::RequestTimeout => Some(903),

            RouterError::GCMAuthentication => Some(904),

            RouterError::Upstream { .. } => None,
        }
    }
}

impl ReportableError for RouterError {
    fn reportable_source(&self) -> Option<&(dyn ReportableError + 'static)> {
        match &self {
            RouterError::Apns(e) => Some(e),
            RouterError::Fcm(e) => Some(e),
            RouterError::SaveDb(e, _) => Some(e),
            _ => None,
        }
    }

    fn is_sentry_event(&self) -> bool {
        match self {
            // apns handle_error emits a metric for ApnsError::Unregistered
            RouterError::Apns(e) => e.is_sentry_event(),
            RouterError::Fcm(e) => e.is_sentry_event(),
            // common handle_error emits metrics for these
            RouterError::Authentication
            | RouterError::GCMAuthentication
            | RouterError::Connect(_)
            | RouterError::NotFound
            | RouterError::RequestTimeout
            | RouterError::TooMuchData(_)
            | RouterError::Upstream { .. } => false,
            RouterError::SaveDb(e, _) => e.is_sentry_event(),
            _ => true,
        }
    }

    fn metric_label(&self) -> Option<&'static str> {
        // NOTE: Some metrics are emitted for other Errors via handle_error
        // callbacks, whereas some are emitted via this method. These 2 should
        // be consolidated: https://mozilla-hub.atlassian.net/browse/SYNC-3695
        match self {
            RouterError::Apns(e) => e.metric_label(),
            RouterError::Fcm(e) => e.metric_label(),
            RouterError::TooMuchData(_) => Some("notification.bridge.error.too_much_data"),
            RouterError::Upstream { status, .. } if status == "RESOURCE_EXHAUSTED" => {
                Some("notification.bridge.error.resource_exhausted")
            }
            _ => None,
        }
    }

    fn extras(&self) -> Vec<(&str, String)> {
        match &self {
            RouterError::Apns(e) => e.extras(),
            RouterError::Fcm(e) => e.extras(),
            RouterError::SaveDb(e, sub) => {
                let mut extras = e.extras();
                if let Some(sub) = sub {
                    extras.append(&mut vec![("sub", sub.clone())]);
                };
                extras
            }
            _ => vec![],
        }
    }
}
