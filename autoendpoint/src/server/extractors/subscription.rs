use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::server::extractors::token_info::{ApiVersion, TokenInfo};
use crate::server::extractors::user::{validate_user, RouterType};
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
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use openssl::hash::MessageDigest;
use std::borrow::Cow;
use uuid::Uuid;

/// Extracts subscription data from `TokenInfo` and verifies auth/crypto headers
#[derive(Clone, Debug)]
pub struct Subscription {
    pub user: DynamoDbUser,
    pub channel_id: Uuid,
    pub router_type: RouterType,
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
                .map_err(ApiErrorKind::Database)?
                .ok_or(ApiErrorKind::NoUser)?;
            let router_type = validate_user(&user, &channel_id, &state).await?;

            // Validate the VAPID JWT token and record the version
            if let Some(vapid) = &vapid {
                validate_vapid_jwt(vapid)?;

                state
                    .metrics
                    .incr(&format!("updates.vapid.draft{:02}", vapid.vapid.version()))?;
            }

            Ok(Subscription {
                user,
                channel_id,
                router_type,
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
    Ok(match vapid.version_data.clone() {
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
        VapidVersionData::Version2 { public_key } => VapidHeaderWithKey { vapid, public_key },
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
    let public_key = &vapid.ok_or(VapidError::MissingKey)?.public_key;

    // Hash the VAPID public key
    let public_key = base64::decode_config(public_key, base64::URL_SAFE_NO_PAD)
        .map_err(|_| VapidError::InvalidKey)?;
    let key_hash = openssl::hash::hash(MessageDigest::sha256(), &public_key)
        .map_err(ApiErrorKind::TokenHashValidation)?;

    // Verify that the VAPID public key equals the (expected) token public key
    if !openssl::memcmp::eq(&key_hash, &token_key) {
        return Err(VapidError::KeyMismatch.into());
    }

    Ok(())
}

/// Validate the VAPID JWT token. Specifically,
/// - Check the signature
/// - Make sure it hasn't expired
/// - Make sure the expiration isn't too far into the future
///
/// This is mostly taken care of by the jsonwebtoken library
fn validate_vapid_jwt(vapid: &VapidHeaderWithKey) -> ApiResult<()> {
    let VapidHeaderWithKey { vapid, public_key } = vapid;

    #[derive(serde::Deserialize)]
    struct Claims {
        exp: u64,
    }

    // Check the signature and make sure the expiration is in the future
    let token_data = jsonwebtoken::decode::<Claims>(
        &vapid.token,
        &DecodingKey::from_ec_der(public_key.as_bytes()),
        &Validation::new(Algorithm::ES256),
    )?;

    // Make sure the expiration isn't too far into the future
    let now = sec_since_epoch();
    const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;

    if token_data.claims.exp - now > ONE_DAY_IN_SECONDS {
        // The expiration is too far in the future
        return Err(VapidError::FutureExpirationToken.into());
    }

    Ok(())
}
