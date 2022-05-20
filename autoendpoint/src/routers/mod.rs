//! Routers route notifications to user agents

use crate::db::error::DbError;
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::adm::error::AdmError;
use crate::routers::apns::error::ApnsError;
use crate::routers::fcm::error::FcmError;
use actix_web::http::StatusCode;
use actix_web::HttpResponse;
use async_trait::async_trait;
use std::collections::HashMap;
use thiserror::Error;

pub mod adm;
pub mod apns;
mod common;
pub mod fcm;
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
#[derive(Debug, PartialEq)]
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
            builder.set_header(key, value);
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

            RouterError::SaveDb(_) => StatusCode::SERVICE_UNAVAILABLE,

            RouterError::GCMAuthentication | // Treat GCM auth failures as special, we reset the user.
            RouterError::UserWasDeleted | RouterError::NotFound => StatusCode::GONE,

            RouterError::TooMuchData(_) => StatusCode::PAYLOAD_TOO_LARGE,

            RouterError::Authentication
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
}
