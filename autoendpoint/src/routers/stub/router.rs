use super::client::StubClient;
use super::error::StubError;
use super::settings::{StubServerSettings, StubSettings};
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::{Router, RouterError, RouterResponse};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Stub router
///
/// This will create a testing router who's behavior is determined
/// by the messages that it handles. This router can be used for
/// local development and testing. The Stub router can be controlled
/// by specifying the action in the `app_id` of the subscription URL.
/// The `success` `app_id` is always defined.
///
/// for example, using a subscription registration URL of
/// `/v1/stub/success/registration` with the request body of
/// `"{\"token\":\"success\", \"key\":"..."}"` will return a successful
/// registration endpoint that uses the provided VAPID key. Calling that endpoint
/// with a VAPID signed request will succeed.
///
/// Likewise, using a subscription registration URL of
/// `/v1/stub/error/registration` with the request body of
/// `"{\"token\":\"General Error\", \"key\":"..."}"` will return that error
/// when the subscription endpoint is called.
pub struct StubRouter {
    settings: StubSettings,
    server_settings: StubServerSettings,
    /// A map from application ID to an authenticated FCM client
    clients: HashMap<String, StubClient>,
}

impl StubRouter {
    /// Create a new `StubRouter`
    pub fn new(settings: StubSettings) -> Result<Self, StubError> {
        let server_settings =
            serde_json::from_str::<StubServerSettings>(&settings.server_credentials)
                .unwrap_or_default();
        let clients = Self::create_clients(&server_settings);
        Ok(Self {
            settings,
            server_settings,
            clients,
        })
    }

    /// Create Test clients for each application. Tests can specify which client
    /// to use by designating the value in the `app_id` of the subscription URL. (e.g.)
    fn create_clients(server_settings: &StubServerSettings) -> HashMap<String, StubClient> {
        let mut clients = HashMap::new();
        // TODO: Expand this to provide for additional error states based on app_id drawn
        // from the StubServerSettings.
        clients.insert("success".to_owned(), StubClient::success());
        clients.insert(
            "error".to_owned(),
            StubClient::error(StubError::General(server_settings.error.clone())),
        );
        clients
    }
}

#[async_trait(?Send)]
impl Router for StubRouter {
    fn register(
        &self,
        router_data_input: &RouterDataInput,
        app_id: &str,
    ) -> Result<HashMap<String, Value>, RouterError> {
        if !self.clients.contains_key(app_id) {
            return Err(
                StubError::General(format!("Unknown App ID: {}", app_id.to_owned())).into(),
            );
        }

        let mut router_data = HashMap::new();
        router_data.insert(
            "token".to_owned(),
            serde_json::to_value(&router_data_input.token).unwrap(),
        );
        router_data.insert("app_id".to_owned(), serde_json::to_value(app_id).unwrap());

        Ok(router_data)
    }

    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        debug!(
            "Sending Test notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(StubError::General("No Registration".to_owned()))?;

        let default = Value::String("success".to_owned());
        let client_type = router_data.get("type").unwrap_or(&default);
        let client = self
            .clients
            .get(&client_type.to_string().to_lowercase())
            .unwrap_or_else(|| self.clients.get("error").unwrap());
        client.call(&self.server_settings).await?;
        Ok(RouterResponse::success(
            format!("{}/m/123", self.settings.url),
            notification.headers.ttl as usize,
        ))
    }
}
