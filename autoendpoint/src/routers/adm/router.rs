use crate::db::client::DbClient;
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::adm::client::AdmClient;
use crate::routers::adm::error::AdmError;
use crate::routers::adm::settings::AdmSettings;
use crate::routers::common::{build_message_data, handle_error, incr_success_metrics};
use crate::routers::{Router, RouterError, RouterResponse};
use async_trait::async_trait;
use cadence::StatsdClient;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

/// 31 days, specified by ADM
const MAX_TTL: usize = 2419200;

/// Amazon Device Messaging router
pub struct AdmRouter {
    settings: AdmSettings,
    endpoint_url: Url,
    metrics: Arc<StatsdClient>,
    ddb: Box<dyn DbClient>,
    /// A map from profile name to an authenticated ADM client
    clients: HashMap<String, AdmClient>,
}

impl AdmRouter {
    /// Create a new `AdmRouter`
    pub fn new(
        settings: AdmSettings,
        endpoint_url: Url,
        http: reqwest::Client,
        metrics: Arc<StatsdClient>,
        ddb: Box<dyn DbClient>,
    ) -> Result<Self, AdmError> {
        let profiles = settings.profiles()?;

        let clients: HashMap<String, AdmClient> = profiles
            .into_iter()
            .map(|(name, profile)| (name, AdmClient::new(&settings, profile, http.clone())))
            .collect();
        trace!("Initialized {} ADM clients", clients.len());

        Ok(Self {
            settings,
            endpoint_url,
            metrics,
            ddb,
            clients,
        })
    }

    /// if we have any clients defined, this connection is "active"
    pub fn active(&self) -> bool {
        !self.clients.is_empty()
    }
}

#[async_trait(?Send)]
impl Router for AdmRouter {
    fn register(
        &self,
        router_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, Value>, RouterError> {
        if !self.clients.contains_key(app_id) {
            return Err(AdmError::InvalidProfile.into());
        }

        let mut router_data = HashMap::new();
        router_data.insert(
            "token".to_string(),
            serde_json::to_value(&router_input.token).unwrap(),
        );
        router_data.insert(
            "creds".to_string(),
            serde_json::json!({ "profile": app_id }),
        );

        Ok(router_data)
    }

    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        debug!(
            "Sending ADM notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(AdmError::NoRegistrationId)?;
        let registration_id = router_data
            .get("token")
            .and_then(Value::as_str)
            .ok_or(AdmError::NoRegistrationId)?;
        let profile = router_data
            .get("creds")
            .and_then(Value::as_object)
            .and_then(|obj| obj.get("profile"))
            .and_then(Value::as_str)
            .ok_or(AdmError::NoProfile)?;
        let ttl = MAX_TTL.min(self.settings.min_ttl.max(notification.headers.ttl as usize));
        let message_data = build_message_data(notification)?;

        // Send the notification to ADM
        let client = self.clients.get(profile).ok_or(AdmError::InvalidProfile)?;
        trace!("Sending message to ADM: {:?}", message_data);
        let new_registration_id = match client
            .send(message_data, registration_id.to_string(), ttl)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                return Err(handle_error(
                    e,
                    &self.metrics,
                    self.ddb.as_ref(),
                    "adm",
                    profile,
                    notification.subscription.user.uaid,
                )
                .await)
            }
        };

        // Sent successfully, update metrics and make response
        trace!("ADM request was successful");
        incr_success_metrics(&self.metrics, "adm", profile, notification);

        // If the returned registration ID is different than the old one, update
        // the user.
        if new_registration_id != registration_id {
            trace!("ADM reports a new registration ID for user, updating our copy");
            let mut user = notification.subscription.user.clone();
            let mut router_data = router_data.clone();

            router_data.insert(
                "token".to_string(),
                serde_json::to_value(&new_registration_id).unwrap(),
            );
            user.router_data = Some(router_data);

            self.ddb.update_user(&mut user).await?;
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
    use crate::db::client::DbClient;
    use crate::db::mock::MockDbClient;
    use crate::error::ApiErrorKind;
    use crate::extractors::routers::RouterType;
    use crate::routers::adm::client::tests::{
        mock_adm_endpoint_builder, mock_token_endpoint, CLIENT_ID, CLIENT_SECRET, REGISTRATION_ID,
    };
    use crate::routers::adm::error::AdmError;
    use crate::routers::adm::router::AdmRouter;
    use crate::routers::adm::settings::AdmSettings;
    use crate::routers::common::tests::{make_notification, CHANNEL_ID};
    use crate::routers::RouterError;
    use crate::routers::{Router, RouterResponse};
    use autopush_common::db::DynamoDbUser;
    use cadence::StatsdClient;
    use mockall::predicate;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Arc;
    use url::Url;

    /// Create a router for testing
    fn make_router(ddb: Box<dyn DbClient>) -> AdmRouter {
        AdmRouter::new(
            AdmSettings {
                base_url: Url::parse(&mockito::server_url()).unwrap(),
                profiles: format!(
                    r#"{{ "dev": {{ "client_id": "{}", "client_secret": "{}" }} }}"#,
                    CLIENT_ID, CLIENT_SECRET
                ),
                ..Default::default()
            },
            Url::parse("http://localhost:8080/").unwrap(),
            reqwest::Client::new(),
            Arc::new(StatsdClient::from_sink("autopush", cadence::NopMetricSink)),
            ddb,
        )
        .unwrap()
    }

    /// Create default user router data
    fn default_router_data() -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        map.insert(
            "token".to_string(),
            serde_json::to_value(REGISTRATION_ID).unwrap(),
        );
        map.insert("creds".to_string(), serde_json::json!({ "profile": "dev" }));
        map
    }

    /// A notification with no data is sent to ADM
    #[tokio::test]
    async fn successful_routing_no_data() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(ddb);
        let _token_mock = mock_token_endpoint();
        let adm_mock = mock_adm_endpoint_builder()
            .match_body(
                serde_json::json!({
                    "data": { "chid": CHANNEL_ID },
                    "expiresAfter": 60
                })
                .to_string()
                .as_str(),
            )
            .with_body(r#"{"registrationID":"test-registration-id"}"#)
            .create();
        let notification = make_notification(default_router_data(), None, RouterType::ADM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok(), "result = {:?}", result);
        assert_eq!(
            result.unwrap(),
            RouterResponse::success("http://localhost:8080/m/test-message-id".to_string(), 0)
        );
        adm_mock.assert();
    }

    /// A notification with data is sent to ADM
    #[tokio::test]
    async fn successful_routing_with_data() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(ddb);
        let _token_mock = mock_token_endpoint();
        let adm_mock = mock_adm_endpoint_builder()
            .match_body(
                serde_json::json!({
                    "data": {
                        "chid": CHANNEL_ID,
                        "body": "test-data",
                        "con": "test-encoding",
                        "enc": "test-encryption",
                        "cryptokey": "test-crypto-key",
                        "enckey": "test-encryption-key"
                    },
                    "expiresAfter": 60
                })
                .to_string()
                .as_str(),
            )
            .with_body(r#"{"registrationID":"test-registration-id"}"#)
            .create();
        let data = "test-data".to_string();
        let notification = make_notification(default_router_data(), Some(data), RouterType::ADM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok(), "result = {:?}", result);
        assert_eq!(
            result.unwrap(),
            RouterResponse::success("http://localhost:8080/m/test-message-id".to_string(), 0)
        );
        adm_mock.assert();
    }

    /// If there is no client for the user's profile, an error is returned and
    /// the ADM request is not sent.
    #[tokio::test]
    async fn missing_client() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(ddb);
        let _token_mock = mock_token_endpoint();
        let adm_mock = mock_adm_endpoint_builder().expect(0).create();
        let mut router_data = default_router_data();
        router_data.insert(
            "creds".to_string(),
            serde_json::json!({ "profile": "unknown-profile" }),
        );
        let notification = make_notification(router_data, None, RouterType::ADM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Adm(AdmError::InvalidProfile))
            ),
            "result = {:?}",
            result
        );
        adm_mock.assert();
    }

    /// If the ADM user no longer exists (404), we drop the user from our database
    #[tokio::test]
    async fn no_adm_user() {
        let notification = make_notification(default_router_data(), None, RouterType::ADM);
        let mut ddb = MockDbClient::new();
        ddb.expect_remove_user()
            .with(predicate::eq(notification.subscription.user.uaid))
            .times(1)
            .return_once(|_| Ok(()));

        let router = make_router(ddb.into_boxed_arc());
        let _token_mock = mock_token_endpoint();
        let _adm_mock = mock_adm_endpoint_builder()
            .with_status(404)
            .with_body(r#"{"reason":"test-message"}"#)
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

    /// If ADM returns a new registration token, update our copy to match
    #[tokio::test]
    async fn update_registration_token() {
        let notification = make_notification(default_router_data(), None, RouterType::ADM);
        let mut ddb = MockDbClient::new();
        ddb.expect_update_user()
            .withf(|user: &DynamoDbUser| {
                user.router_data
                    .as_ref()
                    .unwrap()
                    .get("token")
                    .and_then(Value::as_str)
                    == Some("test-registration-id2")
            })
            .times(1)
            .return_once(|_| Ok(()));

        let router = make_router(ddb.into_boxed_arc());
        let _token_mock = mock_token_endpoint();
        let _adm_mock = mock_adm_endpoint_builder()
            .with_body(r#"{"registrationID":"test-registration-id2"}"#)
            .create();

        let result = router.route_notification(&notification).await;
        assert!(result.is_ok());
    }
}
