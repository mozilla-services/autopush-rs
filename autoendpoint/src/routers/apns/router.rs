use autopush_common::db::client::DbClient;
#[cfg(feature = "reliable_report")]
use autopush_common::reliability::{PushReliability, ReliabilityState};

use crate::error::{ApiError, ApiResult};
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::apns::error::ApnsError;
use crate::routers::apns::settings::{ApnsChannel, ApnsSettings};
use crate::routers::common::{
    build_message_data, incr_error_metric, incr_success_metrics, message_size_check,
};
use crate::routers::{Router, RouterError, RouterResponse};
use a2::{
    self,
    request::payload::{Payload, PayloadLike},
    DefaultNotificationBuilder, Endpoint, NotificationBuilder, NotificationOptions, Priority,
    Response,
};
use actix_web::http::StatusCode;
use async_trait::async_trait;
use cadence::StatsdClient;
use futures::{StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
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
    #[cfg(feature = "reliable_report")]
    reliability: Arc<PushReliability>,
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

/// a2 does not allow for Deserialization of the APS structure.
/// this is copied from that library
#[derive(Deserialize, Serialize, Default, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
#[allow(clippy::upper_case_acronyms)]
pub struct ApsDeser<'a> {
    // The notification content. Can be empty for silent notifications.
    // Note, we overwrite this value, but it's copied and commented here
    // so that future development notes the change.
    // #[serde(skip_serializing_if = "Option::is_none")]
    //pub alert: Option<a2::request::payload::APSAlert<'a>>,
    /// A number shown on top of the app icon.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub badge: Option<u32>,

    /// The name of the sound file to play when user receives the notification.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<&'a str>,

    /// Set to one for silent notifications.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_available: Option<u8>,

    /// When a notification includes the category key, the system displays the
    /// actions for that category as buttons in the banner or alert interface.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<&'a str>,

    /// If set to one, the app can change the notification content before
    /// displaying it to the user.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutable_content: Option<u8>,

    #[serde(skip_serializing_if = "Option::is_none")]
    // Converted for Deserialization
    // pub url_args: Option<&'a [&'a str]>,
    pub url_args: Option<Vec<String>>,
}

#[derive(Default)]
// Replicate a2::request::notification::DefaultAlert
// for lifetime reasons.
pub struct ApsAlertHolder {
    title: String,
    subtitle: String,
    body: String,
    title_loc_key: String,
    title_loc_args: Vec<String>,
    action_loc_key: String,
    loc_key: String,
    loc_args: Vec<String>,
    launch_image: String,
}

impl ApnsRouter {
    /// Create a new APNS router. APNS clients will be initialized for each
    /// channel listed in the settings.
    pub async fn new(
        settings: ApnsSettings,
        endpoint_url: Url,
        metrics: Arc<StatsdClient>,
        db: Box<dyn DbClient>,
        #[cfg(feature = "reliable_report")] reliability: Arc<PushReliability>,
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
            #[cfg(feature = "reliable_report")]
            reliability,
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
        let cert = if !settings.cert.starts_with('-') {
            tokio::fs::read(settings.cert).await?
        } else {
            settings.cert.as_bytes().to_vec()
        };
        let key = if !settings.key.starts_with('-') {
            tokio::fs::read(settings.key).await?
        } else {
            settings.key.as_bytes().to_vec()
        };
        // Timeouts defined in ApnsSettings settings.rs config and can be modified.
        // We define them to prevent possible a2 library changes that could
        // create unexpected behavior if timeouts are altered.
        // They currently map to values matching the detaults in the a2 lib v0.10.
        let apns_settings = ApnsSettings::default();
        let config = a2::ClientConfig {
            endpoint,
            request_timeout_secs: apns_settings.request_timeout_secs,
            pool_idle_timeout_secs: apns_settings.pool_idle_timeout_secs,
        };
        let client = ApnsClientData {
            client: Box::new(
                a2::Client::certificate_parts(&cert, &key, config)
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

    /// Derive an APS message from the replacement JSON block.
    ///
    /// This requires an external "holder" that contains the data that APS will refer to.
    /// The holder should live in the same context as the `aps.build()` method.
    fn derive_aps<'a>(
        &self,
        replacement: Value,
        holder: &'a mut ApsAlertHolder,
    ) -> Result<DefaultNotificationBuilder<'a>, ApnsError> {
        let mut aps = Self::default_aps();
        // a2 does not have a way to bulk replace these values, so do them by hand.
        // these could probably be turned into a macro, but hopefully, this is
        // more one off and I didn't want to fight with the macro generator.
        // This whole thing was included as a byproduct of
        // https://bugzilla.mozilla.org/show_bug.cgi?id=1364403 which was put
        // in place to help debug the iOS build. It was supposed to be temporary,
        // but apparently bit-lock set in and now no one is super sure if it's
        // still needed or used. (I want to get rid of this.)
        if let Some(v) = replacement.get("title") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.title);
                aps = aps.set_title(&holder.title);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("subtitle") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.subtitle);
                aps = aps.set_subtitle(&holder.subtitle);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("body") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.body);
                aps = aps.set_body(&holder.body);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("title_loc_key") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.title_loc_key);
                aps = aps.set_title_loc_key(&holder.title_loc_key);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("title_loc_args") {
            if let Some(v) = v.as_array() {
                let mut args: Vec<String> = Vec::new();
                for val in v {
                    if let Some(value) = val.as_str() {
                        args.push(value.to_owned())
                    } else {
                        return Err(ApnsError::InvalidApsData);
                    }
                }
                holder.title_loc_args = args;
                aps = aps.set_title_loc_args(&holder.title_loc_args);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("action_loc_key") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.action_loc_key);
                aps = aps.set_action_loc_key(&holder.action_loc_key);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("loc_key") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.loc_key);
                aps = aps.set_loc_key(&holder.loc_key);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("loc_args") {
            if let Some(v) = v.as_array() {
                let mut args: Vec<String> = Vec::new();
                for val in v {
                    if let Some(value) = val.as_str() {
                        args.push(value.to_owned())
                    } else {
                        return Err(ApnsError::InvalidApsData);
                    }
                }
                holder.loc_args = args;
                aps = aps.set_loc_args(&holder.loc_args);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        if let Some(v) = replacement.get("launch_image") {
            if let Some(v) = v.as_str() {
                v.clone_into(&mut holder.launch_image);
                aps = aps.set_launch_image(&holder.launch_image);
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        // Honestly, we should just check to see if this is present
        // we don't really care what the value is since we'll never
        // use
        if let Some(v) = replacement.get("mutable-content") {
            if let Some(v) = v.as_i64() {
                if v != 0 {
                    aps = aps.set_mutable_content();
                }
            } else {
                return Err(ApnsError::InvalidApsData);
            }
        }
        Ok(aps)
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

        if let Some(aps) = &router_input.aps {
            if serde_json::from_str::<ApsDeser<'_>>(aps).is_err() {
                return Err(ApnsError::InvalidApsData.into());
            }
            router_data.insert(
                "aps".to_string(),
                serde_json::to_value(aps.clone()).unwrap(),
            );
        }

        Ok(router_data)
    }

    #[allow(unused_mut)]
    async fn route_notification(
        &self,
        mut notification: Notification,
    ) -> ApiResult<RouterResponse> {
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
            .ok_or(ApnsError::NoDeviceToken)?
            .clone();
        let token = router_data
            .get("token")
            .and_then(Value::as_str)
            .ok_or(ApnsError::NoDeviceToken)?;
        let channel = router_data
            .get("rel_channel")
            .and_then(Value::as_str)
            .ok_or(ApnsError::NoReleaseChannel)?;
        let aps_json = router_data.get("aps").cloned();
        let mut message_data = build_message_data(&notification)?;
        message_data.insert("ver", notification.message_id.clone());

        // Get client and build payload
        let ApnsClientData { client, topic } = self
            .clients
            .get(channel)
            .ok_or(ApnsError::InvalidReleaseChannel)?;

        // A simple bucket variable so that I don't have to deal with fun lifetime issues if we need
        // to derive.
        let mut holder = ApsAlertHolder::default();

        // If we are provided a replacement APS block, derive an APS message from it, otherwise
        // start with a blank APS message.
        let aps = if let Some(replacement) = aps_json {
            self.derive_aps(replacement, &mut holder)?
        } else {
            Self::default_aps()
        };

        // Finalize the APS object.
        let mut payload = aps.build(
            token,
            NotificationOptions {
                apns_id: None,
                apns_priority: Some(Priority::High),
                apns_topic: Some(topic),
                apns_collapse_id: None,
                apns_expiration: Some(notification.timestamp + notification.headers.ttl as u64),
                ..Default::default()
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
            #[cfg(feature = "reliable_report")]
            notification
                .record_reliability(
                    &self.reliability,
                    autopush_common::reliability::ReliabilityState::Errored,
                )
                .await;
            return Err(self
                .handle_error(e, notification.subscription.user.uaid, channel)
                .await);
        }

        trace!("APNS request was successful");
        incr_success_metrics(&self.metrics, "apns", channel, &notification);
        #[cfg(feature = "reliable_report")]
        {
            // Record that we've sent the message out to APNS.
            // We can't set the state here because the notification isn't
            // mutable, but we are also essentially consuming the
            // notification nothing else should modify it.
            notification
                .record_reliability(&self.reliability, ReliabilityState::Transmitted)
                .await;
        }

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
    #[cfg(feature = "reliable_report")]
    use autopush_common::reliability::PushReliability;
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
    async fn make_router(client: MockApnsClient, db: Box<dyn DbClient>) -> ApnsRouter {
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
            db: db.clone(),
            #[cfg(feature = "reliable_report")]
            reliability: Arc::new(PushReliability::new(&None, db.clone()).await.unwrap()),
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
        let mdb = MockDbClient::new();
        let db = mdb.into_boxed_arc();
        let router = make_router(client, db).await;
        let notification = make_notification(default_router_data(), None, RouterType::APNS);

        let result = router.route_notification(notification).await;
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
        let mdb = MockDbClient::new();
        let db = mdb.into_boxed_arc();
        let router = make_router(client, db).await;
        let data = "test-data".to_string();
        let notification = make_notification(default_router_data(), Some(data), RouterType::APNS);

        let result = router.route_notification(notification).await;
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
        let router = make_router(client, db).await;
        let mut router_data = default_router_data();
        router_data.insert(
            "rel_channel".to_string(),
            serde_json::to_value("unknown-app-id").unwrap(),
        );
        let notification = make_notification(router_data, None, RouterType::APNS);

        let result = router.route_notification(notification).await;
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
        let router = make_router(client, db.into_boxed_arc()).await;

        let result = router.route_notification(notification).await;
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
        let router = make_router(client, db).await;
        let notification = make_notification(default_router_data(), None, RouterType::APNS);

        let result = router.route_notification(notification).await;
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

    /// An error is returned if the user's APS data is invalid
    #[tokio::test]
    async fn invalid_aps_data() {
        let client = MockApnsClient::new(|_| panic!("The notification should not be sent"));
        let db = MockDbClient::new().into_boxed_arc();
        let router = make_router(client, db).await;
        let mut router_data = default_router_data();
        router_data.insert(
            "aps".to_string(),
            serde_json::json!({"mutable-content": "should be a number"}),
        );
        let notification = make_notification(router_data, None, RouterType::APNS);

        let result = router.route_notification(notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Apns(ApnsError::InvalidApsData))
            ),
            "result = {result:?}"
        );
    }
}
