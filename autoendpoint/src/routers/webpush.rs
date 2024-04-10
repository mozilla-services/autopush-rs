use async_trait::async_trait;
use cadence::{Counted, CountedExt, StatsdClient, Timed};
use reqwest::{Response, StatusCode};
use serde_json::Value;
use std::collections::{hash_map::RandomState, HashMap};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

use crate::error::{ApiErrorKind, ApiResult};
use crate::extractors::{notification::Notification, router_data_input::RouterDataInput};
use crate::routers::{Router, RouterError, RouterResponse};

use autopush_common::db::{client::DbClient, User};

/// The router for desktop user agents.
///
/// These agents are connected via an Autopush connection server. The correct
/// server is located via the database routing table. If the server is busy or
/// not available, the notification is stored in the database.
pub struct WebPushRouter {
    pub db: Box<dyn DbClient>,
    pub metrics: Arc<StatsdClient>,
    pub http: reqwest::Client,
    pub endpoint_url: Url,
}

#[async_trait(?Send)]
impl Router for WebPushRouter {
    fn register(
        &self,
        _router_input: &RouterDataInput,
        _app_id: &str,
    ) -> Result<HashMap<String, Value, RandomState>, RouterError> {
        // WebPush registration happens through the connection server
        Ok(HashMap::new())
    }

    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        // The notification contains the original subscription information
        let user = &notification.subscription.user;
        debug!(
            "✉ Routing WebPush notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("✉ Notification = {:?}", notification);

        // Check if there is a node connected to the client
        if let Some(node_id) = &user.node_id {
            trace!(
                "✉ User has a node ID, sending notification to node: {}",
                &node_id
            );

            // Try to send the notification to the node
            match self.send_notification(notification, node_id).await {
                Ok(response) => {
                    // The node might be busy, make sure it accepted the notification
                    if response.status() == 200 {
                        // The node has received the notification
                        trace!("✉ Node received notification");
                        return Ok(self.make_delivered_response(notification));
                    }

                    trace!(
                        "✉ Node did not receive the notification, response = {:?}",
                        response
                    );
                }
                Err(error) => {
                    if error.is_timeout() {
                        self.metrics.incr("error.node.timeout")?;
                    };
                    if error.is_connect() {
                        self.metrics.incr("error.node.connect")?;
                    }
                    debug!("✉ Error while sending webpush notification: {}", error);
                    self.remove_node_id(user, node_id).await?
                }
            }
        }

        if notification.headers.ttl == 0 {
            let topic = notification.headers.topic.is_some().to_string();
            trace!(
                "✉ Notification has a TTL of zero and was not successfully \
                 delivered, dropping it"
            );
            self.metrics
                .incr_with_tags("notification.message.expired")
                // TODO: include `internal` if meta is set.
                .with_tag("topic", &topic)
                .send();
            return Ok(self.make_delivered_response(notification));
        }

        // Save notification, node is not present or busy
        trace!("✉ Node is not present or busy, storing notification");
        self.store_notification(notification).await?;

        // Retrieve the user data again, they may have reconnected or the node
        // is no longer busy.
        let user = match self.db.get_user(&user.uaid).await {
            Ok(Some(user)) => user,
            Ok(None) => {
                trace!("✉ No user found, must have been deleted");
                return Err(ApiErrorKind::Router(RouterError::UserWasDeleted).into());
            }
            Err(e) => {
                // Database error, but we already stored the message so it's ok
                debug!("✉ Database error while re-fetching user: {}", e);
                return Ok(self.make_stored_response(notification));
            }
        };

        // Try to notify the node the user is currently connected to
        let node_id = match &user.node_id {
            Some(id) => id,
            // The user is not connected to a node, nothing more to do
            None => {
                trace!("✉ User is not connected to a node, returning stored response");
                return Ok(self.make_stored_response(notification));
            }
        };

        // Notify the node to check for messages
        trace!("✉ Notifying node to check for messages");
        match self.trigger_notification_check(&user.uaid, node_id).await {
            Ok(response) => {
                trace!("Response = {:?}", response);
                if response.status() == 200 {
                    trace!("✉ Node has delivered the message");
                    self.metrics
                        .time_with_tags(
                            "notification.total_request_time",
                            (notification.timestamp - autopush_common::util::sec_since_epoch())
                                * 1000,
                        )
                        .with_tag("platform", "websocket")
                        .with_tag("app_id", "direct")
                        .send();

                    Ok(self.make_delivered_response(notification))
                } else {
                    trace!("✉ Node has not delivered the message, returning stored response");
                    Ok(self.make_stored_response(notification))
                }
            }
            Err(error) => {
                // Can't communicate with the node, attempt to stop using it
                debug!("✉ Error while triggering notification check: {}", error);
                self.remove_node_id(&user, node_id).await?;
                Ok(self.make_stored_response(notification))
            }
        }
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
        let notification = notification.serialize_for_delivery();

        self.http.put(&url).json(&notification).send().await
    }

    /// Notify the node to check for notifications for the user
    async fn trigger_notification_check(
        &self,
        uaid: &Uuid,
        node_id: &str,
    ) -> Result<Response, reqwest::Error> {
        let url = format!("{node_id}/notif/{uaid}");

        self.http.put(&url).send().await
    }

    /// Store a notification in the database
    async fn store_notification(&self, notification: &Notification) -> ApiResult<()> {
        self.db
            .save_message(
                &notification.subscription.user.uaid,
                notification.clone().into(),
            )
            .await
            .map_err(|e| ApiErrorKind::Router(RouterError::SaveDb(e)).into())
    }

    /// Remove the node ID from a user. This is done if the user is no longer
    /// connected to the node.
    async fn remove_node_id(&self, user: &User, node_id: &str) -> ApiResult<()> {
        self.metrics.incr("updates.client.host_gone").ok();
        let removed = self
            .db
            .remove_node_id(&user.uaid, node_id, user.connected_at, &user.version)
            .await?;
        if !removed {
            debug!("✉ The node id was not removed");
        }
        Ok(())
    }

    /// Update metrics and create a response for when a notification has been directly forwarded to
    /// an autopush server.
    fn make_delivered_response(&self, notification: &Notification) -> RouterResponse {
        self.make_response(notification, "Direct", StatusCode::CREATED)
    }

    /// Update metrics and create a response for when a notification has been stored in the database
    /// for future transmission.
    fn make_stored_response(&self, notification: &Notification) -> RouterResponse {
        self.make_response(notification, "Stored", StatusCode::CREATED)
    }

    /// Update metrics and create a response after routing a notification
    fn make_response(
        &self,
        notification: &Notification,
        destination_tag: &str,
        status: StatusCode,
    ) -> RouterResponse {
        self.metrics
            .count_with_tags(
                "notification.message_source",
                notification.data.as_ref().map(String::len).unwrap_or(0) as i64,
            )
            .with_tag("destination", destination_tag)
            .send();

        RouterResponse {
            status: actix_http::StatusCode::from_u16(status.as_u16()).unwrap_or_default(),
            headers: {
                let mut map = HashMap::new();
                map.insert(
                    "Location",
                    self.endpoint_url
                        .join(&format!("/m/{}", notification.message_id))
                        .expect("Message ID is not URL-safe")
                        .to_string(),
                );
                map.insert("TTL", notification.headers.ttl.to_string());
                map
            },
            body: None,
        }
    }
}
