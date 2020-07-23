use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::routers::apns::error::ApnsError;
use crate::routers::apns::settings::ApnsSettings;
use crate::routers::{Router, RouterResponse};
use a2::Endpoint;
use async_trait::async_trait;
use cadence::StatsdClient;
use std::collections::HashMap;
use std::fs::File;
use url::Url;

pub struct ApnsRouter {
    /// A map from release channel to APNS client
    clients: HashMap<String, a2::Client>,
    endpoint_url: Url,
    metrics: StatsdClient,
}

impl ApnsRouter {
    pub fn new(
        settings: &ApnsSettings,
        endpoint_url: Url,
        metrics: StatsdClient,
    ) -> Result<Self, ApnsError> {
        let channels = settings.channels()?;

        let mut clients = HashMap::new();
        for (name, settings) in channels {
            let endpoint = if settings.sandbox {
                Endpoint::Sandbox
            } else {
                Endpoint::Production
            };
            let mut certificate = File::open(settings.cert).unwrap();
            let client = a2::Client::certificate(&mut certificate, &settings.password, endpoint)
                .map_err(ApnsError::ApnsClient)?;
            clients.insert(name, client);
        }

        Ok(Self {
            clients,
            endpoint_url,
            metrics,
        })
    }
}

#[async_trait(?Send)]
impl Router for ApnsRouter {
    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        unimplemented!()
    }
}
