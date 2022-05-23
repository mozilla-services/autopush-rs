use crate::db::client::DbClient;
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::extractors::routers::RouterType;
use crate::routers::common::{build_message_data, handle_error, incr_success_metrics};
use crate::routers::fcm::client::FcmClient;
use crate::routers::fcm::error::FcmError;
use crate::routers::fcm::settings::{FcmServerCredential, FcmSettings};
use crate::routers::{Router, RouterError, RouterResponse};
use async_trait::async_trait;
use cadence::StatsdClient;
use serde_json::Value;
use std::collections::HashMap;
use std::str::FromStr;
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
        let server_credentials = settings.credentials()?;
        let clients = Self::create_clients(&settings, server_credentials, http.clone())
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
        server_credentials: HashMap<String, FcmServerCredential>,
        http: reqwest::Client,
    ) -> std::io::Result<HashMap<String, FcmClient>> {
        let mut clients = HashMap::new();

        for (profile, server_credential) in server_credentials {
            clients.insert(
                profile,
                FcmClient::new(settings, server_credential, http.clone()).await?,
            );
        }
        trace!("Initialized {} FCM clients", clients.len());
        Ok(clients)
    }

    /// if we have any clients defined, this connection is "active"
    pub fn active(&self) -> bool {
        !self.clients.is_empty()
    }

    /// Do the gauntlet check to get the routing credentials, these are the
    /// sender/project ID, and the subscription specific user routing token.
    /// FCM stores the values in the top hash as `token` & `app_id`, GCM stores them
    /// in a sub-hash as `creds.auth` and `creds.senderID`.
    /// If any of these error out, it's probably because of a corrupted key.
    fn routing_info(&self, router_data: &HashMap<String, Value>) -> ApiResult<(String, String)> {
        let creds = router_data.get("creds").and_then(Value::as_object);
        // GCM and FCM both should store the client registration_token as token in the router_data.
        // There was some confusion about router table records that may store the client
        // routing token in `creds.auth`, but it's believed that this a duplicate of the
        // server authentication token and can be ignored since we use the value specified
        // in the settings.
        let routing_token = match router_data.get("token").and_then(Value::as_str) {
            Some(v) => v.to_owned(),
            None => {
                return Err(FcmError::NoRegistrationToken.into());
            }
        };
        let app_id = match router_data.get("app_id").and_then(Value::as_str) {
            Some(v) => v.to_owned(),
            None => {
                if creds.is_none() {
                    return Err(FcmError::NoAppId.into());
                }
                match creds
                    .unwrap()
                    .get("senderID")
                    .map(|v| v.as_str())
                    .unwrap_or(None)
                {
                    Some(v) => v.to_owned(),
                    None => return Err(FcmError::NoAppId.into()),
                }
            }
        };
        Ok((routing_token, app_id))
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
            return Err(FcmError::InvalidAppId(app_id.to_owned()).into());
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

        // Older "GCM" set the router data as "senderID" : "auth"
        // Newer "FCM" set the router data as "app_id": "token"
        // The first element is the project identifier, which is
        // matched against the values specified in the settings to
        // get the authentication method. The second is the client
        // provided routing token. This is a proprietary identifier
        // that is sent by the client at registration.
        //
        // Try reading as FCM and fall back to GCM.
        let (routing_token, app_id) = self.routing_info(router_data)?;
        let ttl = MAX_TTL.min(self.settings.min_ttl.max(notification.headers.ttl as usize));
        let message_data = build_message_data(notification)?;

        // Send the notification to FCM
        // (Sigh, errors do not have tags support. )
        let client = self
            .clients
            .get(&app_id)
            .ok_or_else(|| FcmError::InvalidAppId(app_id.clone()))?;

        // GCM is the older message format for android, and it's not possible to generate
        // new test keys. It operates by using a project level bearer token as authorization.
        // FCM operates by using the more complex token as part of an OAuth2
        // transaction. According to the documentation, GCM and FCM are interoperable,
        // meaning that the client provided tokens should match up, and that the structure
        // of the data should also not make a huge difference. (Although, that's untested.)
        //
        match RouterType::from_str(&notification.subscription.user.router_type)
            .unwrap_or(RouterType::FCM)
        {
            RouterType::GCM => {
                trace!("Sending message to GCM: [{:?}]", &app_id);
                if let Err(e) = client.send_gcm(message_data, routing_token, ttl).await {
                    return Err(handle_error(
                        e,
                        &self.metrics,
                        self.ddb.as_ref(),
                        "gcm",
                        &app_id,
                        notification.subscription.user.uaid,
                    )
                    .await);
                }
                incr_success_metrics(&self.metrics, "gcm", &app_id, notification);
            }
            _ => {
                trace!("Sending message to FCM: [{:?}]", &app_id);
                if let Err(e) = client.send(message_data, routing_token, ttl).await {
                    return Err(handle_error(
                        e,
                        &self.metrics,
                        self.ddb.as_ref(),
                        "fcmv1",
                        &app_id,
                        notification.subscription.user.uaid,
                    )
                    .await);
                }
                incr_success_metrics(&self.metrics, "fcmv1", &app_id, notification);
            }
        }
        // Sent successfully, update metrics and make response
        trace!("Send request was successful");

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
        make_service_key, mock_fcm_endpoint_builder, mock_gcm_endpoint_builder,
        mock_token_endpoint, GCM_PROJECT_ID, PROJECT_ID,
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
    async fn make_router(
        fcm_credential: String,
        gcm_credential: String,
        ddb: Box<dyn DbClient>,
    ) -> FcmRouter {
        let url = &mockito::server_url();
        FcmRouter::new(
            FcmSettings {
                base_url: Url::parse(url).unwrap(),
                server_credentials: serde_json::json!({
                    "dev": {
                        "project_id": PROJECT_ID,
                        "credential": fcm_credential
                    },
                    GCM_PROJECT_ID: {
                        "project_id": GCM_PROJECT_ID,
                        "credential": gcm_credential
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

    /// Get the GCM credential data from the database (this needs to read
    /// historic values).
    fn gcm_router_data(credential: String) -> HashMap<String, serde_json::Value> {
        let mut map = HashMap::new();
        let mut creds = HashMap::new();
        map.insert("senderID".to_string(), GCM_PROJECT_ID.to_string());
        creds.insert("creds".to_string(), serde_json::to_value(map).unwrap());
        creds.insert(
            "token".to_string(),
            serde_json::to_value(credential).unwrap(),
        );
        creds
    }

    /// A notification with no data is sent to FCM
    #[tokio::test]
    async fn successful_routing_no_data() {
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(make_service_key(), "whatever".to_string(), ddb).await;
        assert!(router.active());
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder(PROJECT_ID)
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

    /// A notification with no data is sent to GCM if the subscription specifies it.
    #[tokio::test]
    async fn successful_gcm_fallback() {
        let auth_key = "AIzaSyB0ecSrqnEDXQ7yjLXqVc0CUGOeSlq9BsM"; // this is a nonce value used only for testing.
        let registration_id = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        // let project_id = GCM_PROJECT_ID;
        let ddb = MockDbClient::new().into_boxed_arc();
        let router = make_router(make_service_key(), auth_key.to_owned(), ddb).await;
        assert!(router.active());
        // body must match the composed body in `gcm_send` exactly (order, values, etc.)
        let body = serde_json::json!({
            "registration_ids": [registration_id],
            "time_to_live": 60_i32,
            "delay_while_idle": false,
            "data": {
                "chid": CHANNEL_ID
            },
        })
        .to_string();
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_gcm_endpoint_builder()
            .match_header("Authorization", format!("key={}", &auth_key).as_str())
            .match_header("Content-Type", "application/json")
            .with_body(
                r#"{ "multicast_id": 216,"success":1,"failure":0,"canonical_ids":0,"results":[{"message_id":"1:02"}]}"#,
            )
            .match_body(body.as_str())
            .create();
        let notification = make_notification(
            gcm_router_data(registration_id.to_owned()),
            None,
            RouterType::GCM,
        );

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
        let router = make_router(make_service_key(), "whatever".to_string(), ddb).await;
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder(PROJECT_ID)
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
        let router = make_router(make_service_key(), "whatever".to_string(), ddb).await;
        let _token_mock = mock_token_endpoint();
        let fcm_mock = mock_fcm_endpoint_builder(PROJECT_ID).expect(0).create();
        let mut router_data = default_router_data();
        let app_id = "app_id".to_string();
        router_data.insert(
            app_id.clone(),
            serde_json::to_value("unknown-app-id").unwrap(),
        );
        let notification = make_notification(router_data, None, RouterType::FCM);

        let result = router.route_notification(&notification).await;
        assert!(result.is_err());
        assert!(
            matches!(
                &result.as_ref().unwrap_err().kind,
                ApiErrorKind::Router(RouterError::Fcm(FcmError::InvalidAppId(_app_id)))
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

        let router = make_router(
            make_service_key(),
            "whatever".to_string(),
            ddb.into_boxed_arc(),
        )
        .await;
        let _token_mock = mock_token_endpoint();
        let _fcm_mock = mock_fcm_endpoint_builder(PROJECT_ID)
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
