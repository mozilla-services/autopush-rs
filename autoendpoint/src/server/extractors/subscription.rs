use crate::error::ApiError;
use crate::server::extractors::token_info::TokenInfo;
use crate::server::ServerState;
use actix_http::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use futures::future;

/// Extracts subscription data from `TokenInfo` and verifies auth/crypto headers
pub struct Subscription {
    pub uaid: String,
    pub channel_id: String,
    pub api_version: String,
    pub public_key: String,
}

impl FromRequest for Subscription {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, payload: &mut Payload<PayloadStream>) -> Self::Future {
        // Collect token info and server state
        let token_info = match TokenInfo::from_request(req, payload).into_inner() {
            Ok(t) => t,
            Err(e) => return future::err(e),
        };
        let state: Data<ServerState> = Data::extract(req)
            .into_inner()
            .expect("No server state found");
        let fernet = state.fernet.as_ref();

        // Decrypt the token
        let token = match fernet.decrypt(&token_info.token) {
            Ok(t) => t,
            Err(e) => todo!("Error: Invalid token"),
        };

        if token_info.api_version == "v1" && token.len() != 32 {
            todo!("Error: Corrupted push token")
        }

        // Extract public key
        let public_key = "TODO: Extract public key".to_string();
        if let Some(crypto_key_header) = token_info.crypto_key_header {
            todo!("Extract public key from header")
        }

        if let Some(auth_header) = token_info.auth_header {
            todo!("Parse vapid auth")
        }

        // Validate key data if on v2
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
