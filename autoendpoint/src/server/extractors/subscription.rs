use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::server::extractors::token_info::TokenInfo;
use crate::server::headers::crypto_key::CryptoKeyHeader;
use crate::server::headers::vapid::{VapidHeader, VapidVersionData};
use crate::server::{ServerState, VapidError};
use actix_http::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use cadence::{Counted, StatsdClient};
use futures::future::LocalBoxFuture;
use futures::FutureExt;
use openssl::hash;
use std::borrow::Cow;

/// Extracts subscription data from `TokenInfo` and verifies auth/crypto headers
pub struct Subscription {
    pub uaid: String,
    pub channel_id: String,
    pub api_version: String,
    pub public_key: String,
}

impl FromRequest for Subscription {
    type Error = ApiError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        let req = req.clone();

        async move {
            // Collect token info and server state
            let token_info = TokenInfo::extract(&req).await?;
            let state: Data<ServerState> =
                Data::extract(&req).await.expect("No server state found");
            let fernet = state.fernet.as_ref();

            // Decrypt the token
            let token = fernet
                .decrypt(&repad_base64(&token_info.token))
                .map_err(|_| ApiErrorKind::InvalidToken)?;

            // Parse VAPID and extract public key
            let vapid = parse_vapid(&token_info, &state.metrics)?;
            let public_key = extract_public_key(vapid, &token_info)?;

            if token_info.api_version == "v2" {
                version_2_validation(&token, &public_key)?;
            } else {
                version_1_validation(&token)?;
            }

            Ok(Subscription {
                uaid: hex::encode(&token[..16]),
                channel_id: hex::encode(&token[16..32]),
                api_version: token_info.api_version,
                public_key,
            })
        }
        .boxed_local()
    }
}

/// Add back padding to a base64 string
fn repad_base64(data: &str) -> Cow<'_, str> {
    let remaining_padding = data.len() % 4;

    if remaining_padding != 0 {
        let mut data = data.to_string();

        for _ in 0..remaining_padding {
            data.push('=');
        }

        Cow::Owned(data)
    } else {
        Cow::Borrowed(data)
    }
}

/// Parse the authorization header for VAPID data and update metrics
fn parse_vapid(token_info: &TokenInfo, metrics: &StatsdClient) -> ApiResult<VapidHeader> {
    let auth_header = token_info
        .auth_header
        .as_ref()
        .ok_or(VapidError::MissingToken)?;
    let vapid = VapidHeader::parse(auth_header)?;

    metrics
        .incr_with_tags("notification.auth")
        .with_tag("vapid", &vapid.version().to_string())
        .with_tag("scheme", &vapid.scheme)
        .send();

    Ok(vapid)
}

/// Extract the VAPID public key from the headers
fn extract_public_key(vapid: VapidHeader, token_info: &TokenInfo) -> ApiResult<String> {
    Ok(match vapid.version_data {
        VapidVersionData::Version1 => {
            // VAPID v1 stores the public key in the Crypto-Key header
            token_info
                .crypto_key_header
                .as_deref()
                .and_then(CryptoKeyHeader::parse)
                .and_then(|crypto_keys| crypto_keys.get_by_key("p256ecdsa").map(str::to_string))
                .ok_or(ApiErrorKind::InvalidCryptoKey)?
        }
        VapidVersionData::Version2 { public_key } => public_key,
    })
}

/// `/webpush/v1/` validations
fn version_1_validation(token: &[u8]) -> ApiResult<()> {
    if token.len() != 32 {
        // Corrupted token
        return Err(ApiErrorKind::InvalidToken.into());
    }

    Ok(())
}

/// `/webpush/v2/` validations
fn version_2_validation(token: &[u8], public_key: &str) -> ApiResult<()> {
    if token.len() != 64 {
        // Corrupted token
        return Err(ApiErrorKind::InvalidToken.into());
    }

    // Verify that the sender is authorized to send notifications.
    // The last 32 bytes of the token is the hashed public key.
    let token_key = &token[32..];

    // Hash the VAPID public key
    let public_key = base64::decode_config(public_key, base64::URL_SAFE_NO_PAD)
        .map_err(|_| VapidError::InvalidKey)?;
    let key_hash = hash::hash(hash::MessageDigest::sha256(), &public_key)?;

    // Verify that the VAPID public key equals the (expected) token public key
    if !openssl::memcmp::eq(&key_hash, &token_key) {
        return Err(VapidError::KeyMismatch.into());
    }

    Ok(())
}
