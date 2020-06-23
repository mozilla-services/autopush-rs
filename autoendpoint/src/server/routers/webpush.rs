//! The router for desktop user agents.
//!
//! These agents are connected via an Autopush connection server. The correct
//! server is located via the database routing table. If the server is busy or
//! not available, the notification is stored in the database.

use crate::error::ApiResult;
use crate::server::extractors::notification::Notification;
use crate::server::routers::{Router, RouterResponse};
use async_trait::async_trait;
use autopush_common::db::DynamoStorage;
use cadence::{Counted, StatsdClient};
use reqwest::{Response, StatusCode};
use std::collections::HashMap;

struct WebPushRouter {
    ddb: DynamoStorage,
    metrics: StatsdClient,
    http: reqwest::Client,
}

#[async_trait]
impl Router for WebPushRouter {
    async fn route_notification(&self, notification: Notification) -> ApiResult<RouterResponse> {
        // Check if node_id is present
        // - Send Notification to node
        //   - Success: Done, return 200
        //   - Error (Node busy): Jump to Save notification below
        //   - Error (Client gone, node gone/dead): Clear node entry for user
        //       - Both: Done, return 503

        if let Some(node_id) = &notification.subscription.user.node_id {
            match self.send_notification(&notification, node_id).await {
                Ok(response) => {
                    if response.status() == 200 {
                        return Ok(self.make_delivered_response(&notification));
                    }
                }
                Err(error) => {
                    debug!("Error while sending webpush notification: {}", error);
                    self.metrics.incr("updates.client.host_gone").ok();

                    // todo: clear node_id
                }
            }
        }

        // Save notification, node is not present or busy
        // - Save notification
        //   - Success (older version): Done, return 202
        //   - Error (db error): Done, return 503

        // - Lookup client again to get latest node state after save.
        //   - Success (node found): Notify node of new notification
        //     - Success: Done, return 200
        //     - Error (no client): Done, return 202
        //     - Error (no node): Clear node entry
        //       - Both: Done, return 202
        //   - Success (no node): Done, return 202
        //   - Error (db error): Done, return 202
        //   - Error (no client) : Done, return 404

        unimplemented!()
    }
}

impl WebPushRouter {
    /// Send the notification to the node
    async fn send_notification(
        &self,
        notification: &Notification,
        node_id: &str,
    ) -> Result<Response, reqwest::Error> {
        let url = format!("{}/push/{}", node_id, notification.subscription.user.uaid);
        let notification = autopush_common::notification::Notification::from(notification.clone());

        self.http.put(&url).json(&notification).send().await
    }

    /// Update metrics and create a response for when a notification has been directly forwarded to
    /// an autopush server.
    fn make_delivered_response(&self, notification: &Notification) -> RouterResponse {
        self.metrics
            .count_with_tags(
                "notification.message_data",
                notification.data.as_ref().map(String::len).unwrap_or(0) as i64,
            )
            .with_tag("destination", "Direct")
            .send();

        RouterResponse {
            status: StatusCode::OK,
            headers: {
                let mut map = HashMap::new();
                // TODO: get endpoint url and add message_id to it
                map.insert("Location", "TODO".to_string());
                map.insert("TTL", notification.headers.ttl.to_string());
                map
            },
            body: None,
        }
    }
}
