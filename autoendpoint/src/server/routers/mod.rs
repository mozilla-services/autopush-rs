//! Routers route notifications to user agents

use crate::error::ApiResult;
use crate::server::extractors::notification::Notification;

pub mod webpush;

pub trait Router {
    /// Route a notification to the user
    fn route_notification(&self, notification: Notification) -> ApiResult<()>;
}
