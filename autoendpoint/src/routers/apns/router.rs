use crate::error::ApiResult;
use crate::extractors::notification::Notification;
use crate::routers::apns::error::ApnsError;
use crate::routers::apns::settings::{ApnsChannelSettings, ApnsSettings};
use crate::routers::{Router, RouterResponse};
use a2::Endpoint;
use async_trait::async_trait;
use cadence::StatsdClient;
use futures::{StreamExt, TryStreamExt};
use std::collections::HashMap;
use url::Url;

pub struct ApnsRouter {
    /// A map from release channel to APNS client
    clients: HashMap<String, a2::Client>,
    endpoint_url: Url,
    metrics: StatsdClient,
}

impl ApnsRouter {
    /// Create a new APNS router. APNS clients will be initialized for each
    /// channel listed in the settings.
    pub async fn new(
        settings: &ApnsSettings,
        endpoint_url: Url,
        metrics: StatsdClient,
    ) -> Result<Self, ApnsError> {
        let channels = settings.channels()?;

        let clients = futures::stream::iter(channels)
            .then(|(name, settings)| Self::create_client(name, settings))
            .try_collect()
            .await?;

        Ok(Self {
            clients,
            endpoint_url,
            metrics,
        })
    }

    /// Create an APNS client for the channel
    async fn create_client(
        name: String,
        settings: ApnsChannelSettings,
    ) -> Result<(String, a2::Client), ApnsError> {
        let endpoint = if settings.sandbox {
            Endpoint::Sandbox
        } else {
            Endpoint::Production
        };
        let cert = tokio::fs::read(settings.cert).await?;
        let key = tokio::fs::read(settings.key).await?;
        let client =
            a2::Client::certificate_parts(&cert, &key, endpoint).map_err(ApnsError::ApnsClient)?;

        Ok((name, client))
    }
}

#[async_trait(?Send)]
impl Router for ApnsRouter {
    async fn route_notification(&self, notification: &Notification) -> ApiResult<RouterResponse> {
        debug!(
            "Sending APNS notification to UAID {}",
            notification.subscription.user.uaid
        );
        trace!("Notification = {:?}", notification);

        let router_data = notification
            .subscription
            .user
            .router_data
            .as_ref()
            .ok_or(ApnsError::NoDeviceToken)?;

        unimplemented!()
    }
}
