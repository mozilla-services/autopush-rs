use crate::db::client::DbClient;
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::common::{build_message_data, handle_error, incr_success_metrics};
use crate::routers::fcm::client::FcmClient;
use crate::routers::fcm::error::FcmError;
use crate::routers::fcm::settings::{FcmCredential, FcmSettings};
use crate::routers::{Router, RouterError, RouterResponse};
use async_trait::async_trait;
use cadence::StatsdClient;
use serde_json::Value;
use std::collections::HashMap;
use url::Url;

/// 28 days
const MAX_TTL: usize = 28 * 24 * 60 * 60;

/// Firebase Cloud Messaging router
pub struct FcmRouter {
    settings: FcmSettings,
    endpoint_url: Url,
    metrics: StatsdClient,
    ddb: Box<dyn DbClient>,
    /// A map from application ID to an authenticated FCM client
    clients: HashMap<String, FcmClient>,
}

impl FcmRouter {
    /// Create a new `FcmRouter`
    pub async fn new(
        settings: FcmSettings,
        endpoint_url: Url,
        http: reqwest::Client,
        metrics: StatsdClient,
        ddb: Box<dyn DbClient>,
    ) -> Result<Self, FcmError> {
        let credentials = settings.credentials()?;
        let clients = Self::create_clients(&settings, credentials, http.clone())
            .await
            .map_err(FcmError::OAuthClientBuild)?;
        Ok(Self {
            settings,
            endpoint_url,
            metrics,
            ddb,
            clients,
        })
    }

    /// Create FCM clients for each application
    async fn create_clients(
        settings: &FcmSettings,
        credentials: HashMap<String, FcmCredential>,
        http: reqwest::Client,
    ) -> std::io::Result<HashMap<String, FcmClient>> {
        let mut clients = HashMap::new();

        for (profile, credential) in credentials {
            clients.insert(
                profile,
                FcmClient::new(settings, credential, http.clone()).await?,
            );
        }
        trace!("Initialized {} FCM clients", clients.len());
        Ok(clients)
    }

    /// if we have any clients defined, this connection is "active"
    pub fn active(&self) -> bool {
        !self.clients.is_empty()
    }
}

#[async_trait(?Send)]
impl Router for FcmRouter {
    fn register(
        &self,
        router_data_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, Value>, RouterError> {
        if !self.clients.contains_key(app_id) {
            return Err(FcmError::InvalidAppId.into());
        }

        let mut router_data = HashMap::new();
        router_data.insert(
            "token".to_string(),
            serde_json::to_value(&router_data_input.token).unwrap(),
        );
        router_data.insert("app_id".to_string(), serde_json::to_value(app_id).unwrap());

        Ok(router_data)
    }

    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        debug!(
            "Sending FCM notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(FcmError::NoRegistrationToken)?;
        let fcm_token = router_data
            .get("token")
            .and_then(Value::as_str)
            .ok_or(FcmError::NoRegistrationToken)?;
        let app_id = router_data
            .get("app_id")
            .and_then(Value::as_str)
            .ok_or(FcmError::NoAppId)?;
        let ttl = MAX_TTL.min(self.settings.min_ttl.max(notification.headers.ttl as usize));
        let message_data = build_message_data(notification)?;

        // Send the notification to FCM
        let client = self.clients.get(app_id).ok_or(FcmError::InvalidAppId)?;
        trace!("Sending message to FCM: {:?}", message_data);
        if let Err(e) = client.send(message_data, fcm_token.to_string(), ttl).await {
            return Err(handle_error(
                e,
                &self.metrics,
                self.ddb.as_ref(),
                "fcmv1",
                app_id,
                notification.subscription.user.uaid,
            )
            .await);
        }

        // Sent successfully, update metrics and make response
        trace!("FCM request was successful");
        incr_success_metrics(&self.metrics, "fcmv1", app_id, notification);

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
    use crate::db::client::DbClient;
    use crate::db::mock::MockDbClient;
    use crate::error::ApiErrorKind;
    use crate::extractors::routers::RouterType;
    use crate::routers::common::tests::{make_notification, CHANNEL_ID};
    use crate::routers::fcm::client::tests::{
        make_service_key, mock_fcm_endpoint_builder, mock_token_endpoint, PROJECT_ID,
    };
    use crate::routers::fcm::error::FcmError;
    use crate::routers::fcm::router::FcmRouter;
    use crate::routers::fcm::settings::FcmSettings;
    use crate::routers::RouterError;
    use crate::routers::{Router, RouterResponse};

    use cadence::StatsdClient;
    use mockall::predicate;
    use std::collections::HashMap;
    use url::Url;

    const FCM_TOKEN: &str = "test-token";

    /// Create a router for testing, using the given service auth file
    async fn make_router(credential: String, ddb: Box<dyn DbClient>) -> FcmRouter {
        FcmRouter::new(
            FcmSettings {
                base_url: Url::parse(&mockito::server_url()).unwrap(),
                credentials: serde_json::json!({
                    "dev": {
                        "project_id": PROJECT_ID,
                        "credential": credential
                    }
                })
                .to_string(),
                ..Default::default()
            },
            Url::parse("http://localhost:8080/").unwrap(),
            reqwest::Client::new(),
            StatsdClient::from_sink("autopush", cadence::NopMetricSink),
            ddb,
        )
        .await
        .unwrap()
    }

    /// Create default user router data
    fn default_router_data() -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert(
            "token".to_string(),
            serde_json::to_value(FCM_TOKEN).unwrap(),
        );
        map.insert("app_id".to_string(), serde_json::to_value("dev").unwrap());
        map
    }

    /// A notification with no data is sent to FCM
    #[tokio::test]
    async fn successful_routing_no_data() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(String::from_utf8(make_service_key()).unwrap(), ddb).await;
        assert!(router.active());
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder()
            .match_body(
                serde_json::json!({
                    "message": {
                        "android": {
                            "data": {
                                "chid": CHANNEL_ID
                            },
                            "ttl": "60s"
                        },
                        "token": "test-token"
                    }
                })
                .to_string()
                .as_str(),
            )
            .create();
        let notification = make_notification(default_router_data(), None, RouterType::FCM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok(), "result = {:?}", result);
        assert_eq!(
            result.unwrap(),
            RouterResponse::success("http://localhost:8080/m/test-message-id".to_string(), 0)
        );
        fcm_mock.assert();
    }

    /// A notification with data is sent to FCM
    #[tokio::test]
    async fn successful_routing_with_data() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(String::from_utf8(make_service_key()).unwrap(), ddb).await;
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder()
            .match_body(
                serde_json::json!({
                    "message": {
                        "android": {
                            "data": {
                                "chid": CHANNEL_ID,
                                "body": "test-data",
                                "con": "test-encoding",
                                "enc": "test-encryption",
                                "cryptokey": "test-crypto-key",
                                "enckey": "test-encryption-key"
                            },
                            "ttl": "60s"
                        },
                        "token": "test-token"
                    }
                })
                .to_string()
                .as_str(),
            )
            .create();
        let data = "test-data".to_string();
        let notification = make_notification(default_router_data(), Some(data), RouterType::FCM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok(), "result = {:?}", result);
        assert_eq!(
            result.unwrap(),
            RouterResponse::success("http://localhost:8080/m/test-message-id".to_string(), 0)
        );
        fcm_mock.assert();
    }

    /// If there is no client for the user's app ID, an error is returned and
    /// the FCM request is not sent.
    #[tokio::test]
    async fn missing_client() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(String::from_utf8(make_service_key()).unwrap(), ddb).await;
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder().expect(0).create();
        let mut router_data = default_router_data();
        router_data.insert(
            "app_id".to_string(),
            serde_json::to_value("unknown-app-id").unwrap(),
        );
        let notification = make_notification(router_data, None, RouterType::FCM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Fcm(FcmError::InvalidAppId))
            ),
            "result = {:?}",
            result
        );
        fcm_mock.assert();
    }

    /// If the FCM user no longer exists (404), we drop the user from our database
    #[tokio::test]
    async fn no_fcm_user() {
        let notification = make_notification(default_router_data(), None, RouterType::FCM);
        let mut ddb = MockDbClient::new();
        ddb.expect_remove_user()
            .with(predicate::eq(notification.subscription.user.uaid))
            .times(1)
            .return_once(|_| Ok(()));

        let key = String::from_utf8(make_service_key()).unwrap();
        let router = make_router(key, ddb.into_boxed_arc()).await;
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_fcm_endpoint_builder()
            .with_status(404)
            .with_body(r#"{"error":{"status":"NOT_FOUND","message":"test-message"}}"#)
            .create();

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::NotFound)
            ),
            "result = {:?}",
            result
        );
    }
}
