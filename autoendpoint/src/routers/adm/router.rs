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
use url::Url;

/// 31 days, specified by ADM
const MAX_TTL: usize = 2419200;

/// Amazon Device Messaging router
pub struct AdmRouter {
    settings: AdmSettings,
    endpoint_url: Url,
    metrics: StatsdClient,
    ddb: Box<dyn DbClient>,
    /// A map from profile name to an authenticated ADM client
    clients: HashMap<String, AdmClient>,
}

impl AdmRouter {
    /// Create a new `AdmRouter`
    pub async fn new(
        settings: AdmSettings,
        endpoint_url: Url,
        http: reqwest::Client,
        metrics: StatsdClient,
        ddb: Box<dyn DbClient>,
    ) -> Result<Self, AdmError> {
        let profiles = settings.profiles()?;
        let clients = profiles
            .into_iter()
            .map(|(name, profile)| (name, AdmClient::new(&settings, profile, http.clone())))
            .collect();

        Ok(Self {
            settings,
            endpoint_url,
            metrics,
            ddb,
            clients,
        })
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
        let message_data = build_message_data(notification, self.settings.max_data)?;

        // Send the notification to ADM
        let client = self.clients.get(profile).ok_or(AdmError::InvalidProfile)?;
        trace!("Sending message to ADM: {:?}", message_data);
        if let Err(e) = client
            .send(message_data, registration_id.to_string(), ttl)
            .await
        {
            return Err(handle_error(
                e,
                &self.metrics,
                self.ddb.as_ref(),
                "adm",
                profile,
                notification.subscription.user.uaid,
            )
            .await);
        }

        // Sent successfully, update metrics and make response
        trace!("ADM request was successful");
        incr_success_metrics(&self.metrics, "adm", profile, notification);

        Ok(RouterResponse::success(
            self.endpoint_url
                .join(&format!("/m/{}", notification.message_id))
                .expect("Message ID is not URL-safe")
                .to_string(),
            notification.headers.ttl as usize,
        ))
    }
}
