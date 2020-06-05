use crate::error::ApiError;
use actix_http::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future;

/// Extracts basic token data from the webpush request path and headers
pub struct TokenInfo {
    pub api_version: String,
    pub token: String,
    pub crypto_key_header: Option<String>,
    pub auth_header: Option<String>,
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
        let crypto_key_header = req
            .headers()
            .get("crypto-key")
            .and_then(|h| h.to_str().ok())
            .map(str::to_string);
        let auth_header = req
            .headers()
            .get("authorization")
            .and_then(|h| h.to_str().ok())
            .map(str::to_string);

        future::ok(TokenInfo {
            api_version,
            token,
            crypto_key_header,
            auth_header,
        })
    }
}
