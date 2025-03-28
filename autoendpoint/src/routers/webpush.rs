use async_trait::async_trait;
#[cfg(feature = "reliable_report")]
use autopush_common::reliability::PushReliability;
use cadence::{Counted, CountedExt, StatsdClient, Timed};
use reqwest::{Response, StatusCode};
use serde_json::Value;
use std::collections::{hash_map::RandomState, HashMap};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::extractors::{notification::Notification, router_data_input::RouterDataInput};
use crate::headers::vapid::VapidHeaderWithKey;
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
    #[cfg(feature = "reliable_report")]
    pub reliability: Arc<PushReliability>,
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

    async fn route_notification(
        &self,
        mut notification: Notification,
    ) -> ApiResult<RouterResponse> {
        // The notification contains the original subscription information
        let user = &notification.subscription.user.clone();
        // A clone of the notification used only for the responses
        // The canonical Notification is consumed by the various functions.
        debug!(
            "✉ Routing WebPush notification to UAID {} :: {:?}",
            notification.subscription.user.uaid, notification.subscription.reliability_id,
        );
        trace!("✉ Notification = {:?}", notification);

        // Check if there is a node connected to the client
        if let Some(node_id) = &user.node_id {
            trace!(
                "✉ User has a node ID, sending notification to node: {}",
                &node_id
            );

            #[cfg(feature = "reliable_report")]
            let revert_state = notification.reliable_state;
            #[cfg(feature = "reliable_report")]
            notification
                .record_reliability(
                    &self.reliability,
                    autopush_common::reliability::ReliabilityState::IntTransmitted,
                )
                .await;
            match self.send_notification(&notification, node_id).await {
                Ok(response) => {
                    // The node might be busy, make sure it accepted the notification
                    if response.status() == 200 {
                        // The node has received the notification
                        trace!("✉ Node received notification");
                        return Ok(self.make_delivered_response(&notification));
                    }
                    trace!(
                        "✉ Node did not receive the notification, response = {:?}",
                        response
                    );
                }
                Err(error) => {
                    if let ApiErrorKind::ReqwestError(error) = &error.kind {
                        if error.is_timeout() {
                            self.metrics.incr("error.node.timeout")?;
                        };
                        if error.is_connect() {
                            self.metrics.incr("error.node.connect")?;
                        };
                    };
                    debug!("✉ Error while sending webpush notification: {}", error);
                    self.remove_node_id(user, node_id).await?
                }
            }

            #[cfg(feature = "reliable_report")]
            // Couldn't send the message! So revert to the prior state if we have one
            if let Some(revert_state) = revert_state {
                trace!(
                    "🔎⚠️ Revert {:?} from {:?} to {:?}",
                    &notification.reliability_id,
                    &notification.reliable_state,
                    revert_state
                );
                notification
                    .record_reliability(&self.reliability, revert_state)
                    .await;
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
            #[cfg(feature = "reliable_report")]
            notification
                .record_reliability(
                    &self.reliability,
                    autopush_common::reliability::ReliabilityState::Expired,
                )
                .await;
            return Ok(self.make_delivered_response(&notification));
        }

        // Save notification, node is not present or busy
        trace!("✉ Node is not present or busy, storing notification");
        self.store_notification(&mut notification).await?;

        // Retrieve the user data again, they may have reconnected or the node
        // is no longer busy.
        let user = match self.db.get_user(&user.uaid).await {
            Ok(Some(user)) => user,
            Ok(None) => {
                trace!("✉ No user found, must have been deleted");
                return Err(self.handle_error(
                    ApiErrorKind::Router(RouterError::UserWasDeleted),
                    notification.subscription.vapid.clone(),
                ));
            }
            Err(e) => {
                // Database error, but we already stored the message so it's ok
                debug!("✉ Database error while re-fetching user: {}", e);
                return Ok(self.make_stored_response(&notification));
            }
        };

        // Try to notify the node the user is currently connected to
        let node_id = match &user.node_id {
            Some(id) => id,
            // The user is not connected to a node, nothing more to do
            None => {
                trace!("✉ User is not connected to a node, returning stored response");
                return Ok(self.make_stored_response(&notification));
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

                    Ok(self.make_delivered_response(&notification))
                } else {
                    trace!("✉ Node has not delivered the message, returning stored response");
                    Ok(self.make_stored_response(&notification))
                }
            }
            Err(error) => {
                // Can't communicate with the node, attempt to stop using it
                debug!("✉ Error while triggering notification check: {}", error);
                self.remove_node_id(&user, node_id).await?;
                Ok(self.make_stored_response(&notification))
            }
        }
    }
}

impl WebPushRouter {
    /// Use the same sort of error chokepoint that all the mobile clients use.
    fn handle_error(&self, error: ApiErrorKind, vapid: Option<VapidHeaderWithKey>) -> ApiError {
        let mut err = ApiError::from(error);
        if let Some(Ok(claims)) = vapid.map(|v| v.vapid.claims()) {
            let mut extras = err.extras.unwrap_or_default();
            if let Some(sub) = claims.sub {
                extras.extend([("sub".to_owned(), sub)]);
            }
            err.extras = Some(extras);
        };
        err
    }

    /// Consume and send the notification to the node
    async fn send_notification(
        &self,
        notification: &Notification,
        node_id: &str,
    ) -> ApiResult<Response> {
        let url = format!("{}/push/{}", node_id, notification.subscription.user.uaid);

        let notification_out = notification.serialize_for_delivery()?;

        trace!(
            "⏩ out: Notification: {}, channel_id: {} :: {:?}",
            &notification.subscription.user.uaid,
            &notification.subscription.channel_id,
            &notification_out,
        );
        Ok(self.http.put(&url).json(&notification_out).send().await?)
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
    async fn store_notification(&self, notification: &mut Notification) -> ApiResult<()> {
        let result = self
            .db
            .save_message(
                &notification.subscription.user.uaid,
                notification.clone().into(),
            )
            .await
            .map_err(|e| {
                self.handle_error(
                    ApiErrorKind::Router(RouterError::SaveDb(
                        e,
                        // try to extract the `sub` from the VAPID claims.
                        notification.subscription.vapid.as_ref().map(|vapid| {
                            vapid
                                .vapid
                                .claims()
                                .ok()
                                .and_then(|c| c.sub)
                                .unwrap_or_default()
                        }),
                    )),
                    notification.subscription.vapid.clone(),
                )
            });
        #[cfg(feature = "reliable_report")]
        notification
            .record_reliability(
                &self.reliability,
                autopush_common::reliability::ReliabilityState::Stored,
            )
            .await;
        result
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
                "notification.message_data",
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

#[cfg(test)]
mod test {
    use std::boxed::Box;
    use std::sync::Arc;

    use reqwest;

    use crate::extractors::subscription::tests::{make_vapid, PUB_KEY};
    use crate::headers::vapid::VapidClaims;
    use autopush_common::errors::ReportableError;
    #[cfg(feature = "reliable_report")]
    use autopush_common::reliability::PushReliability;

    use super::*;
    use autopush_common::db::mock::MockDbClient;

    fn make_router(db: Box<dyn DbClient>) -> WebPushRouter {
        WebPushRouter {
            db: db.clone(),
            metrics: Arc::new(StatsdClient::from_sink("autopush", cadence::NopMetricSink)),
            http: reqwest::Client::new(),
            endpoint_url: Url::parse("http://localhost:8080/").unwrap(),
            #[cfg(feature = "reliable_report")]
            reliability: Arc::new(PushReliability::new(&None, db).unwrap()),
        }
    }

    #[tokio::test]
    async fn pass_extras() {
        let db = MockDbClient::new().into_boxed_arc();
        let router = make_router(db);
        let sub = "foo@example.com";
        let vapid = make_vapid(
            sub,
            "https://push.services.mozilla.org",
            VapidClaims::default_exp(),
            PUB_KEY.to_owned(),
        );

        let err = router.handle_error(ApiErrorKind::LogCheck, Some(vapid));
        assert!(err.extras().contains(&("sub", sub.to_owned())));
    }
}
