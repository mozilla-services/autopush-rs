use crate::error::ApiError;
use crate::server::extractors::subscription::Subscription;
use crate::server::extractors::webpush_headers::WebPushHeaders;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use autopush_common::util::sec_since_epoch;
use futures::{future, FutureExt, StreamExt};

/// Extracts notification data from `Subscription` and request data
pub struct Notification {
    pub uaid: String,
    pub channel_id: String,
    pub version: String,
    pub ttl: Option<u64>,
    pub topic: Option<String>,
    pub timestamp: u64,
    pub data: String,
}

impl FromRequest for Notification {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, payload: &mut Payload<PayloadStream>) -> Self::Future {
        let req = req.clone();
        let mut payload = payload.take();

        async move {
            let headers = WebPushHeaders::from_request(&req, &mut payload).await?;
            let subscription = Subscription::from_request(&req, &mut payload).await?;

            // Read data and convert to base64
            let mut data = Vec::new();
            while let Some(item) = payload.next().await {
                data.extend_from_slice(&item.map_err(|_| ApiError::Internal("todo".to_string()))?);
            }
            let data = base64::encode_config(data, base64::URL_SAFE_NO_PAD);

            Ok(Notification {
                uaid: subscription.uaid,
                channel_id: subscription.channel_id,
                version: subscription.api_version,
                ttl: headers.ttl,
                topic: headers.topic,
                timestamp: sec_since_epoch(),
                data,
            })
        }
        .boxed_local()
    }
}
