use crate::error::{ApiError, ApiErrorKind};
use crate::server::extractors::subscription::Subscription;
use crate::server::extractors::webpush_headers::WebPushHeaders;
use crate::server::ServerState;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use autopush_common::util::sec_since_epoch;
use futures::{future, FutureExt, StreamExt};

/// Extracts notification data from `Subscription` and request data
pub struct Notification {
    pub subscription: Subscription,
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
            let headers = WebPushHeaders::extract(&req).await?;
            let subscription = Subscription::extract(&req).await?;
            let state = Data::<ServerState>::extract(&req)
                .await
                .expect("No server state found");

            // Read data and convert to base64
            let mut data = Vec::new();
            while let Some(item) = payload.next().await {
                data.extend_from_slice(&item.map_err(ApiErrorKind::PayloadError)?);

                // Make sure the payload isn't too big
                let max_bytes = state.settings.max_data_bytes;
                if data.len() > max_bytes {
                    return Err(ApiErrorKind::PayloadTooLarge(max_bytes).into());
                }
            }
            let data = base64::encode_config(data, base64::URL_SAFE_NO_PAD);

            Ok(Notification {
                subscription,
                ttl: headers.ttl,
                topic: headers.topic,
                timestamp: sec_since_epoch(),
                data,
            })
        }
        .boxed_local()
    }
}
