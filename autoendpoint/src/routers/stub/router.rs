use super::client::StubClient;
use super::error::StubError;
use super::settings::StubSettings;
use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::extractors::router_data_input::RouterDataInput;
use crate::routers::{Router, RouterError, RouterResponse};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;

/// Firebase Cloud Messaging router
pub struct StubRouter {
    settings: StubSettings,
    /// A map from application ID to an authenticated FCM client
    clients: HashMap<String, StubClient>,
}

impl StubRouter {
    /// Create a new `StubRouter`
    pub fn new(settings: StubSettings) -> Result<Self, StubError> {
        let clients = Self::create_clients();
        Ok(Self { settings, clients })
    }

    /// Create Test clients for each application
    fn create_clients() -> HashMap<String, StubClient> {
        let mut clients = HashMap::new();

        clients.insert("success".to_owned(), StubClient::success());
        clients.insert(
            "error".to_owned(),
            StubClient::error(StubError::General("General Error".to_owned())),
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
            "token".to_string(),
            serde_json::to_value(&router_data_input.token).unwrap(),
        );
        router_data.insert("app_id".to_string(), serde_json::to_value(app_id).unwrap());

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
            .unwrap_or_else(|| self.clients.get("success").unwrap());
        client.call().await?;
        Ok(RouterResponse::success(
            format!("{}/m/123", self.settings.url),
            notification.headers.ttl as usize,
        ))
    }
}
