use crate::error::ApiError;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest, HttpResponse};
use futures::future;

/// Handle the `/wpush/{api_version}/{token}` and `/wpush/{token}` routes
pub async fn webpush_route() -> HttpResponse {
    HttpResponse::Ok().finish()
}

pub struct TokenInfo {
    api_version: String,
    token: String,
    crypto_key: String,
    auth_header: String,
}

impl FromRequest for TokenInfo {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        // Path variables
        let api_version = req
            .match_info()
            .get("api_version")
            .unwrap_or("v1")
            .to_string();
        let token = req
            .match_info()
            .get("token")
            .expect("{token} must be part of the webpush path")
            .to_string();

        // Headers
        let crypto_key = req
            .headers()
            .get("crypto-key")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_string();
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default()
            .to_string();

        future::ok(TokenInfo {
            api_version,
            token,
            crypto_key,
            auth_header,
        })
    }
}

pub struct Subscription {
    uaid: String,
    channel_id: String,
    version: String,
    public_key: String,
}

pub struct Notification {
    channel_id: String,
    version: String,
    ttl: String,
    topic: String,
    timestamp: String,
    data: String,
    headers: String,
}
