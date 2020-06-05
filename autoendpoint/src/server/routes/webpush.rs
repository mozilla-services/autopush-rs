use crate::error::ApiError;
use crate::server::ServerState;
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
    crypto_key_header: Option<String>,
    auth_header: Option<String>,
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

pub struct Subscription {
    uaid: String,
    channel_id: String,
    api_version: String,
    public_key: String,
}

impl FromRequest for Subscription {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, payload: &mut Payload<PayloadStream>) -> Self::Future {
        let token_info = match TokenInfo::from_request(req, payload).into_inner() {
            Ok(t) => t,
            Err(e) => return future::err(e),
        };
        let state = req
            .app_data::<ServerState>()
            .expect("No server state found");
        let fernet = state.fernet.as_ref();

        let token = match fernet.decrypt(&token_info.token) {
            Ok(t) => t,
            Err(e) => todo!("Error: Invalid token"),
        };
        let public_key = "TODO: Extract public key".to_string();

        if let Some(crypto_key_header) = token_info.crypto_key_header {
            todo!("Extract public key from header")
        }

        if let Some(auth_header) = token_info.auth_header {
            todo!("Parse vapid auth")
        }

        if token_info.api_version == "v1" && token.len() != 32 {
            todo!("Error: Corrupted push token")
        }

        if token_info.api_version == "v2" {
            todo!("Perform v2 checks")
        }

        future::ok(Subscription {
            uaid: hex::encode(&token[..16]),
            channel_id: hex::encode(&token[16..32]),
            api_version: token_info.api_version,
            public_key,
        })
    }
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
