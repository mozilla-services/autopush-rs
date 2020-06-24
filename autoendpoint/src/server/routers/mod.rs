//! Routers route notifications to user agents

use crate::error::ApiResult;
use crate::server::extractors::notification::Notification;
use actix_web::http::StatusCode;
use actix_web::HttpResponse;
use async_trait::async_trait;
use std::collections::HashMap;

pub mod webpush;

#[async_trait(?Send)]
pub trait Router {
    /// Route a notification to the user
    async fn route_notification(&self, notification: Notification) -> ApiResult<RouterResponse>;
}

/// The response returned when a router routes a notification
pub struct RouterResponse {
    pub status: StatusCode,
    pub headers: HashMap<&'static str, String>,
    pub body: Option<String>,
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
