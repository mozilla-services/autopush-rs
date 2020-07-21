//! Routers route notifications to user agents

use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::routers::fcm::error::FcmError;
use actix_web::http::StatusCode;
use actix_web::HttpResponse;
use async_trait::async_trait;
use std::collections::HashMap;
use thiserror::Error;

pub mod fcm;
pub mod webpush;

#[async_trait(?Send)]
pub trait Router {
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
    Fcm(#[from] FcmError),

    #[error("Database error while saving notification")]
    SaveDb(#[source] autopush_common::errors::Error),

    #[error("User was deleted during routing")]
    UserWasDeleted,
}

impl RouterError {
    /// Get the associated HTTP status code
    pub fn status(&self) -> StatusCode {
        match self {
            RouterError::Fcm(e) => e.status(),
            RouterError::SaveDb(_) => StatusCode::SERVICE_UNAVAILABLE,
            RouterError::UserWasDeleted => StatusCode::GONE,
        }
    }

    /// Get the associated error number
    pub fn errno(&self) -> Option<usize> {
        match self {
            RouterError::Fcm(e) => e.errno(),
            RouterError::SaveDb(_) => Some(201),
            RouterError::UserWasDeleted => Some(105),
        }
    }
}
