use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::server::extractors::token_info::{ApiVersion, TokenInfo};
use crate::server::extractors::user::validate_user;
use crate::server::headers::crypto_key::CryptoKeyHeader;
use crate::server::headers::vapid::{VapidHeader, VapidHeaderWithKey, VapidVersionData};
use crate::server::{ServerState, VapidError};
use actix_http::{Payload, PayloadStream};
use actix_web::web::Data;
use actix_web::{FromRequest, HttpRequest};
use autopush_common::db::DynamoDbUser;
use autopush_common::util::sec_since_epoch;
use cadence::{Counted, StatsdClient};
use futures::compat::Future01CompatExt;
use futures::future::LocalBoxFuture;
use futures::FutureExt;
use openssl::hash;
use std::borrow::Cow;
use std::collections::HashMap;
use uuid::Uuid;

/// Extracts subscription data from `TokenInfo` and verifies auth/crypto headers
pub struct Subscription {
    pub user: DynamoDbUser,
    pub channel_id: Uuid,
    pub vapid: Option<VapidHeaderWithKey>,
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

            // Decrypt the token
            let token = state
                .fernet
                .decrypt(&repad_base64(&token_info.token))
                .map_err(|_| ApiErrorKind::InvalidToken)?;

            // Parse VAPID and extract public key.
            let vapid: Option<VapidHeaderWithKey> = parse_vapid(&token_info, &state.metrics)?
                .map(|vapid| extract_public_key(vapid, &token_info))
                .transpose()?;

            match token_info.api_version {
                ApiVersion::Version1 => version_1_validation(&token)?,
                ApiVersion::Version2 => version_2_validation(&token, vapid.as_ref())?,
            }

            // Load and validate user data
            let uaid = Uuid::from_slice(&token[..16])?;
            let channel_id = Uuid::from_slice(&token[16..32])?;
            let user = state
                .ddb
                .get_user(&uaid)
                .compat()
                .await
                .map_err(ApiErrorKind::Database)?;
            validate_user(&user, &channel_id, &state).await?;

            Ok(Subscription {
                user,
                channel_id,
                vapid,
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
fn parse_vapid(token_info: &TokenInfo, metrics: &StatsdClient) -> ApiResult<Option<VapidHeader>> {
    let auth_header = match token_info.auth_header.as_ref() {
        Some(header) => header,
        None => return Ok(None),
    };

    let vapid = VapidHeader::parse(auth_header)?;

    metrics
        .incr_with_tags("notification.auth")
        .with_tag("vapid", &vapid.version().to_string())
        .with_tag("scheme", &vapid.scheme)
        .send();

    Ok(Some(vapid))
}

/// Extract the VAPID public key from the headers
fn extract_public_key(vapid: VapidHeader, token_info: &TokenInfo) -> ApiResult<VapidHeaderWithKey> {
    Ok(match &vapid.version_data {
        VapidVersionData::Version1 => {
            // VAPID v1 stores the public key in the Crypto-Key header
            let header = token_info.crypto_key_header.as_deref().ok_or_else(|| {
                ApiErrorKind::InvalidEncryption("Missing Crypto-Key header".to_string())
            })?;
            let header_data = CryptoKeyHeader::parse(header).ok_or_else(|| {
                ApiErrorKind::InvalidEncryption("Invalid Crypto-Key header".to_string())
            })?;
            let public_key = header_data.get_by_key("p256ecdsa").ok_or_else(|| {
                ApiErrorKind::InvalidEncryption(
                    "Missing p256ecdsa in Crypto-Key header".to_string(),
                )
            })?;

            VapidHeaderWithKey {
                vapid,
                public_key: public_key.to_string(),
            }
        }
        VapidVersionData::Version2 { public_key } => VapidHeaderWithKey {
            public_key: public_key.clone(),
            vapid,
        },
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
fn version_2_validation(token: &[u8], vapid: Option<&VapidHeaderWithKey>) -> ApiResult<()> {
    if token.len() != 64 {
        // Corrupted token
        return Err(ApiErrorKind::InvalidToken.into());
    }

    // Verify that the sender is authorized to send notifications.
    // The last 32 bytes of the token is the hashed public key.
    let token_key = &token[32..];
    let public_key = &vapid
        .ok_or(ApiErrorKind::VapidError(VapidError::MissingKey))?
        .public_key;

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
