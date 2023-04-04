use autopush_common::db::client::DbClient;

use crate::error::{ApiError, ApiResult};
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::apns::error::ApnsError;
use crate::routers::apns::settings::{ApnsChannel, ApnsSettings};
use crate::routers::common::{
    build_message_data, incr_error_metric, incr_success_metrics, message_size_check,
};
use crate::routers::{Router, RouterError, RouterResponse};
use a2::request::payload::Payload;
use a2::{
    self, DefaultNotificationBuilder, Endpoint, NotificationBuilder, NotificationOptions, Priority,
    Response,
};
use actix_web::http::StatusCode;
use async_trait::async_trait;
use bytebuffer::ByteBuffer;
use cadence::StatsdClient;
use futures::{StreamExt, TryStreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

/// Apple Push Notification Service router
pub struct ApnsRouter {
    /// A map from release channel to APNS client
    clients: HashMap<String, ApnsClientData>,
    settings: ApnsSettings,
    endpoint_url: Url,
    metrics: Arc<StatsdClient>,
    db: Box<dyn DbClient>,
}

struct ApnsClientData {
    client: Box<dyn ApnsClient>,
    topic: String,
}

#[async_trait]
trait ApnsClient: Send + Sync {
    async fn send(&self, payload: Payload<'_>) -> Result<a2::Response, a2::Error>;
}

#[async_trait]
impl ApnsClient for a2::Client {
    async fn send(&self, payload: Payload<'_>) -> Result<Response, a2::Error> {
        self.send(payload).await
    }
}

impl ApnsRouter {
    /// Create a new APNS router. APNS clients will be initialized for each
    /// channel listed in the settings.
    pub async fn new(
        settings: ApnsSettings,
        endpoint_url: Url,
        metrics: Arc<StatsdClient>,
        db: Box<dyn DbClient>,
    ) -> Result<Self, ApnsError> {
        let channels = settings.channels()?;

        let clients: HashMap<String, ApnsClientData> = futures::stream::iter(channels)
            .then(|(name, settings)| Self::create_client(name, settings))
            .try_collect()
            .await?;

        trace!("Initialized {} APNs clients", clients.len());
        Ok(Self {
            clients,
            settings,
            endpoint_url,
            metrics,
            db,
        })
    }

    /// Create an APNS client for the channel
    async fn create_client(
        name: String,
        settings: ApnsChannel,
    ) -> Result<(String, ApnsClientData), ApnsError> {
        let endpoint = if settings.sandbox {
            Endpoint::Sandbox
        } else {
            Endpoint::Production
        };
        let mut cert = ByteBuffer::from_vec(if !settings.cert.starts_with('-') {
            tokio::fs::read(settings.cert).await?
        } else {
            settings.cert.as_bytes().to_vec()
        });
        let key = String::from_utf8(if !settings.key.starts_with('-') {
            tokio::fs::read(settings.key).await?
        } else {
            settings.key.as_bytes().to_vec()
        })
        .map_err(|e| ApnsError::Config("Bad Key".to_owned(), e.to_string()))?;
        let client = ApnsClientData {
            client: Box::new(
                a2::Client::certificate(&mut cert, &key, endpoint)
                    .map_err(ApnsError::ApnsClient)?,
            ),
            topic: settings
                .topic
                .unwrap_or_else(|| format!("com.mozilla.org.{name}")),
        };

        Ok((name, client))
    }

    /// The default APS data for a notification
    fn default_aps<'a>() -> DefaultNotificationBuilder<'a> {
        DefaultNotificationBuilder::new()
            .set_title_loc_key("SentTab.NoTabArrivingNotification.title")
            .set_loc_key("SentTab.NoTabArrivingNotification.body")
            .set_mutable_content()
    }

    /// Handle an error by logging, updating metrics, etc
    async fn handle_error(&self, error: a2::Error, uaid: Uuid, channel: &str) -> ApiError {
        match &error {
            a2::Error::ResponseError(response) => {
                // capture the APNs error as a metric response. This allows us to spot trends.
                // While APNS can return a number of errors (see a2::response::ErrorReason) we
                // shouldn't encounter many of those.
                let reason = response
                    .error
                    .as_ref()
                    .map(|r| format!("{:?}", r.reason))
                    .unwrap_or_else(|| "Unknown".to_owned());
                let code = StatusCode::from_u16(response.code).unwrap_or(StatusCode::BAD_GATEWAY);
                incr_error_metric(&self.metrics, "apns", channel, &reason, code, None);
                if response.code == 410 {
                    debug!("APNS recipient has been unregistered, removing user");
                    if let Err(e) = self.db.remove_user(&uaid).await {
                        warn!("Error while removing user due to APNS 410: {}", e);
                    }

                    return ApiError::from(ApnsError::Unregistered);
                } else {
                    warn!("APNS error: {:?}", response.error);
                }
            }
            a2::Error::ConnectionError(e) => {
                error!("APNS connection error: {:?}", e);
                incr_error_metric(
                    &self.metrics,
                    "apns",
                    channel,
                    "connection_unavailable",
                    StatusCode::SERVICE_UNAVAILABLE,
                    None,
                );
            }
            _ => {
                warn!("Unknown error while sending APNS request: {}", error);
                incr_error_metric(
                    &self.metrics,
                    "apns",
                    channel,
                    "unknown",
                    StatusCode::BAD_GATEWAY,
                    None,
                );
            }
        }

        ApiError::from(ApnsError::ApnsUpstream(error))
    }

    /// if we have any clients defined, this connection is "active"
    pub fn active(&self) -> bool {
        !self.clients.is_empty()
    }
}

#[async_trait(?Send)]
impl Router for ApnsRouter {
    fn register(
        &self,
        router_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, Value>, RouterError> {
        if !self.clients.contains_key(app_id) {
            return Err(ApnsError::InvalidReleaseChannel.into());
        }

        let mut router_data = HashMap::new();
        router_data.insert(
            "token".to_string(),
            serde_json::to_value(&router_input.token).unwrap(),
        );
        router_data.insert(
            "rel_channel".to_string(),
            serde_json::to_value(app_id).unwrap(),
        );

        Ok(router_data)
    }

    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        debug!(
            "Sending APNS notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        // Build message data
        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(ApnsError::NoDeviceToken)?;
        let token = router_data
            .get("token")
            .and_then(Value::as_str)
            .ok_or(ApnsError::NoDeviceToken)?;
        let channel = router_data
            .get("rel_channel")
            .and_then(Value::as_str)
            .ok_or(ApnsError::NoReleaseChannel)?;
        let mut message_data = build_message_data(notification)?;
        message_data.insert("ver", notification.message_id.clone());

        // Get client and build payload
        let ApnsClientData { client, topic } = self
            .clients
            .get(channel)
            .ok_or(ApnsError::InvalidReleaseChannel)?;
        let mut payload = Self::default_aps().build(
            token,
            NotificationOptions {
                apns_id: None,
                apns_priority: Some(Priority::High),
                apns_topic: Some(topic),
                apns_collapse_id: None,
                apns_expiration: Some(notification.timestamp + notification.headers.ttl as u64),
            },
        );
        payload.data = message_data
            .into_iter()
            .map(|(k, v)| (k, Value::String(v)))
            .collect();

        // Check size limit
        let payload_json = payload
            .clone()
            .to_json_string()
            .map_err(ApnsError::SizeLimit)?;
        message_size_check(payload_json.as_bytes(), self.settings.max_data)?;

        // Send to APNS
        trace!("Sending message to APNS: {:?}", payload);
        if let Err(e) = client.send(payload).await {
            return Err(self
                .handle_error(e, notification.subscription.user.uaid, channel)
                .await);
        }

        // Sent successfully, update metrics and make response
        trace!("APNS request was successful");
        incr_success_metrics(&self.metrics, "apns", channel, notification);

        Ok(RouterResponse::success(
            self.endpoint_url
                .join(&format!("/m/{}", notification.message_id))
                .expect("Message ID is not URL-safe")
                .to_string(),
            notification.headers.ttl as usize,
        ))
    }
}

#[cfg(test)]
mod tests {
    use crate::error::ApiErrorKind;
    use crate::extractors::routers::RouterType;
    use crate::routers::apns::error::ApnsError;
    use crate::routers::apns::router::{ApnsClient, ApnsClientData, ApnsRouter};
    use crate::routers::apns::settings::ApnsSettings;
    use crate::routers::common::tests::{make_notification, CHANNEL_ID};
    use crate::routers::{Router, RouterError, RouterResponse};
    use a2::request::payload::Payload;
    use a2::{Error, Response};
    use async_trait::async_trait;
    use autopush_common::db::client::DbClient;
    use autopush_common::db::mock::MockDbClient;
    use cadence::StatsdClient;
    use mockall::predicate;
    use std::collections::HashMap;
    use std::sync::Arc;
    use url::Url;

    const DEVICE_TOKEN: &str = "test-token";
    const APNS_ID: &str = "deadbeef-4f5e-4403-be8f-35d0251655f5";

    #[allow(clippy::type_complexity)]
    /// A mock APNS client which allows one to supply a custom APNS response/error
    struct MockApnsClient {
        send_fn: Box<dyn Fn(Payload<'_>) -> Result<a2::Response, a2::Error> + Send + Sync>,
    }

    #[async_trait]
    impl ApnsClient for MockApnsClient {
        async fn send(&self, payload: Payload<'_>) -> Result<Response, Error> {
            (self.send_fn)(payload)
        }
    }

    impl MockApnsClient {
        fn new<F>(send_fn: F) -> Self
        where
            F: Fn(Payload<'_>) -> Result<a2::Response, a2::Error>,
            F: Send + Sync + 'static,
        {
            Self {
                send_fn: Box::new(send_fn),
            }
        }
    }

    /// Create a successful APNS response
    fn apns_success_response() -> a2::Response {
        a2::Response {
            error: None,
            apns_id: Some(APNS_ID.to_string()),
            code: 200,
        }
    }

    /// Create a router for testing, using the given APNS client
    fn make_router(client: MockApnsClient, db: Box<dyn DbClient>) -> ApnsRouter {
        ApnsRouter {
            clients: {
                let mut map = HashMap::new();
                map.insert(
                    "test-channel".to_string(),
                    ApnsClientData {
                        client: Box::new(client),
                        topic: "test-topic".to_string(),
                    },
                );
                map
            },
            settings: ApnsSettings::default(),
            endpoint_url: Url::parse("http://localhost:8080/").unwrap(),
            metrics: Arc::new(StatsdClient::from_sink("autopush", cadence::NopMetricSink)),
            db,
        }
    }

    /// Create default user router data
    fn default_router_data() -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert(
            "token".to_string(),
            serde_json::to_value(DEVICE_TOKEN).unwrap(),
        );
        map.insert(
            "rel_channel".to_string(),
            serde_json::to_value("test-channel").unwrap(),
        );
        map
    }

    /// A notification with no data is packaged correctly and sent to APNS
    #[tokio::test]
    async fn successful_routing_no_data() {
        use a2::NotificationBuilder;

        let client = MockApnsClient::new(|payload| {
            let built = ApnsRouter::default_aps().build(DEVICE_TOKEN, Default::default());
            assert_eq!(
                serde_json::to_value(payload.aps).unwrap(),
                serde_json::to_value(built.aps).unwrap()
            );
            assert_eq!(payload.device_token, DEVICE_TOKEN);
            assert_eq!(payload.options.apns_topic, Some("test-topic"));
            assert_eq!(
                serde_json::to_value(payload.data).unwrap(),
                serde_json::json!({
                    "chid": CHANNEL_ID,
                    "ver": "test-message-id"
                })
            );

            Ok(apns_success_response())
        });
        let db = MockDbClient::new().into_boxed_arc();
        let router = make_router(client, db);
        let notification = make_notification(default_router_data(), None, RouterType::APNS);

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok(), "result = {result:?}");
        assert_eq!(
            result.unwrap(),
            RouterResponse::success("http://localhost:8080/m/test-message-id".to_string(), 0)
        );
    }

    /// A notification with data is packaged correctly and sent to APNS
    #[tokio::test]
    async fn successful_routing_with_data() {
        use a2::NotificationBuilder;

        let client = MockApnsClient::new(|payload| {
            let built = ApnsRouter::default_aps().build(DEVICE_TOKEN, Default::default());
            assert_eq!(serde_json::json!(payload.aps), serde_json::json!(built.aps));
            assert_eq!(payload.device_token, DEVICE_TOKEN);
            assert_eq!(payload.options.apns_topic, Some("test-topic"));
            assert_eq!(
                serde_json::to_value(payload.data).unwrap(),
                serde_json::json!({
                    "chid": CHANNEL_ID,
                    "ver": "test-message-id",
                    "body": "test-data",
                    "con": "test-encoding",
                    "enc": "test-encryption",
                    "cryptokey": "test-crypto-key",
                    "enckey": "test-encryption-key"
                })
            );

            Ok(apns_success_response())
        });
        let db = MockDbClient::new().into_boxed_arc();
        let router = make_router(client, db);
        let data = "test-data".to_string();
        let notification = make_notification(default_router_data(), Some(data), RouterType::APNS);

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok(), "result = {result:?}");
        assert_eq!(
            result.unwrap(),
            RouterResponse::success("http://localhost:8080/m/test-message-id".to_string(), 0)
        );
    }

    /// If there is no client for the user's release channel, an error is
    /// returned and the APNS request is not sent.
    #[tokio::test]
    async fn missing_client() {
        let client = MockApnsClient::new(|_| panic!("The notification should not be sent"));
        let db = MockDbClient::new().into_boxed_arc();
        let router = make_router(client, db);
        let mut router_data = default_router_data();
        router_data.insert(
            "rel_channel".to_string(),
            serde_json::to_value("unknown-app-id").unwrap(),
        );
        let notification = make_notification(router_data, None, RouterType::APNS);

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Apns(ApnsError::InvalidReleaseChannel))
            ),
            "result = {result:?}"
        );
    }

    /// If APNS says the user doesn't exist anymore, we return a specific error
    /// and remove the user from the database.
    #[tokio::test]
    async fn user_not_found() {
        let client = MockApnsClient::new(|_| {
            Err(a2::Error::ResponseError(a2::Response {
                error: Some(a2::ErrorBody {
                    reason: a2::ErrorReason::Unregistered,
                    timestamp: Some(0),
                }),
                apns_id: None,
                code: 410,
            }))
        });
        let notification = make_notification(default_router_data(), None, RouterType::APNS);
        let mut db = MockDbClient::new();
        db.expect_remove_user()
            .with(predicate::eq(notification.subscription.user.uaid))
            .times(1)
            .return_once(|_| Ok(()));
        let router = make_router(client, db.into_boxed_arc());

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Apns(ApnsError::Unregistered))
            ),
            "result = {result:?}"
        );
    }

    /// APNS errors (other than Unregistered) are wrapped and returned
    #[tokio::test]
    async fn upstream_error() {
        let client = MockApnsClient::new(|_| {
            Err(a2::Error::ResponseError(a2::Response {
                error: Some(a2::ErrorBody {
                    reason: a2::ErrorReason::BadCertificate,
                    timestamp: None,
                }),
                apns_id: None,
                code: 403,
            }))
        });
        let db = MockDbClient::new().into_boxed_arc();
        let router = make_router(client, db);
        let notification = make_notification(default_router_data(), None, RouterType::APNS);

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Apns(ApnsError::ApnsUpstream(
                    a2::Error::ResponseError(a2::Response {
                        error: Some(a2::ErrorBody {
                            reason: a2::ErrorReason::BadCertificate,
                            timestamp: None,
                        }),
                        apns_id: None,
                        code: 403,
                    })
                )))
            ),
            "result = {result:?}"
        );
    }
}
