//! The router for desktop user agents.
//!
//! These agents are connected via an Autopush connection server. The correct
//! server is located via the database routing table. If the server is busy or
//! not available, the notification is stored in the database.

use crate::error::ApiResult;
use crate::server::extractors::notification::Notification;
use crate::server::routers::Router;
use autopush_common::db::DynamoStorage;
use cadence::StatsdClient;

struct WebPushRouter {
    ddb: DynamoStorage,
    metrics: StatsdClient,
}

impl Router for WebPushRouter {
    fn route_notification(&self, notification: Notification) -> ApiResult<()> {
        unimplemented!()
    }
}
