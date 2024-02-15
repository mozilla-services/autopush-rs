use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use actix_web::{dev::Payload, web::Data, FromRequest, HttpRequest};
use autopush_common::{
    db::User,
    tags::Tags,
    util::{b64_decode_std, b64_decode_url, sec_since_epoch},
};
use cadence::{CountedExt, StatsdClient};
use futures::{future::LocalBoxFuture, FutureExt};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use openssl::hash::MessageDigest;
use serde::{Deserialize, Serialize};
use url::Url;
use uuid::Uuid;

use crate::error::{ApiError, ApiErrorKind, ApiResult};
use crate::extractors::{
    token_info::{ApiVersion, TokenInfo},
    user::validate_user,
};
use crate::headers::{
    crypto_key::CryptoKeyHeader,
    vapid::{VapidError, VapidHeader, VapidHeaderWithKey, VapidVersionData},
};
use crate::metrics::Metrics;
use crate::server::AppState;

const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;

/// Extracts subscription data from `TokenInfo` and verifies auth/crypto headers
#[derive(Clone, Debug)]
pub struct Subscription {
    pub user: User,
    pub channel_id: Uuid,
    pub vapid: Option<VapidHeaderWithKey>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VapidClaims {
    exp: u64,
    aud: String,
    sub: String,
}

impl Default for VapidClaims {
    fn default() -> Self {
        Self {
            exp: sec_since_epoch() + ONE_DAY_IN_SECONDS,
            aud: "No audience".to_owned(),
            sub: "No sub".to_owned(),
        }
    }
}

impl fmt::Display for VapidClaims {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VapidClaims")
            .field("exp", &self.exp)
            .field("aud", &self.aud)
            .field("sub", &self.sub)
            .finish()
    }
}
impl FromRequest for Subscription {
    type Error = ApiError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        let req = req.clone();

        async move {
            // Collect token info and server state
            let token_info = TokenInfo::extract(&req).await?;
            trace!("🔐 Token info: {:?}", &token_info);
            let app_state: Data<AppState> =
                Data::extract(&req).await.expect("No server state found");
            let metrics = Metrics::from(&app_state);

            // Decrypt the token
            let token = app_state
                .fernet
                .decrypt(&repad_base64(&token_info.token))
                .map_err(|e| {
                    error!("🔐 fernet: {:?}", e);
                    ApiErrorKind::InvalidToken
                })?;

            // Parse VAPID and extract public key.
            let vapid: Option<VapidHeaderWithKey> = parse_vapid(&token_info, &app_state.metrics)?
                .map(|vapid| extract_public_key(vapid, &token_info))
                .transpose()?;

            trace!("raw vapid: {:?}", &vapid);

            // Capturing the vapid sub right now will cause too much cardinality. Instead,
            // let's just capture if we have a valid VAPID, as well as what sort of bad sub
            // values we get.
            if let Some(header) = vapid.clone() {
                let sub = header
                    .vapid
                    .sub()
                    .map_err(|e| {
                        // Capture the type of error and add it to metrics.
                        let mut tags = Tags::default();
                        tags.tags
                            .insert("error".to_owned(), e.as_metric().to_owned());
                        metrics
                            .clone()
                            .incr_with_tags("notification.auth.error", Some(tags));
                    })
                    .unwrap_or_default();
                // For now, record that we had a good (?) VAPID sub,
                metrics.clone().incr("notification.auth.ok");
                info!("VAPID sub: {:?}", sub)
            };

            match token_info.api_version {
                ApiVersion::Version1 => version_1_validation(&token)?,
                ApiVersion::Version2 => version_2_validation(&token, vapid.as_ref())?,
            }

            // Load and validate user data.
            // Note: It is safe to unwrap the Uuid result because an error is
            // only returned if the slice length is not 16.
            let uaid = Uuid::from_slice(&token[..16]).unwrap();
            let channel_id = Uuid::from_slice(&token[16..32]).unwrap();

            trace!("UAID: {:?}, CHID: {:?}", uaid, channel_id);

            let user = app_state
                .db
                .get_user(&uaid)
                .await?
                .ok_or(ApiErrorKind::NoSubscription)?;

            trace!("user: {:?}", &user);
            validate_user(&user, &channel_id, &app_state).await?;

            // Validate the VAPID JWT token and record the version
            if let Some(vapid) = &vapid {
                validate_vapid_jwt(vapid, &app_state.settings.endpoint_url(), &metrics)?;

                app_state
                    .metrics
                    .incr(&format!("updates.vapid.draft{:02}", vapid.vapid.version()))?;
            }

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
    let trailing_chars = data.len() % 4;

    if trailing_chars != 0 {
        let mut data = data.to_string();

        for _ in trailing_chars..4 {
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

    let vapid = VapidHeader::parse(auth_header).map_err(|e| {
        metrics
            .incr_with_tags("notification.auth.error")
            .with_tag("error", e.as_metric())
            .send();
        e
    })?;

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

/// Decode a public key string
///
/// NOTE: Some customers send a VAPID public key with incorrect padding and
/// in standard base64 encoding. (Both of these violate the VAPID RFC)
/// Prior python versions ignored these errors, so we should too.
fn decode_public_key(public_key: &str) -> ApiResult<Vec<u8>> {
    if public_key.contains(['/', '+']) {
        b64_decode_std(public_key.trim_end_matches('='))
    } else {
        b64_decode_url(public_key.trim_end_matches('='))
    }
    .map_err(|e| {
        error!("decode_public_key: {:?}", e);
        VapidError::InvalidKey(e.to_string()).into()
    })
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
    let public_key = decode_public_key(public_key)?;
    let key_hash = openssl::hash::hash(MessageDigest::sha256(), &public_key)
        .map_err(ApiErrorKind::TokenHashValidation)?;

    // Verify that the VAPID public key equals the (expected) token public key
    if !openssl::memcmp::eq(&key_hash, token_key) {
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
fn validate_vapid_jwt(
    vapid: &VapidHeaderWithKey,
    domain: &Url,
    metrics: &Metrics,
) -> ApiResult<()> {
    let VapidHeaderWithKey { vapid, public_key } = vapid;

    let public_key = decode_public_key(public_key)?;
    let token_data = match jsonwebtoken::decode::<VapidClaims>(
        &vapid.token,
        &DecodingKey::from_ec_der(&public_key),
        &Validation::new(Algorithm::ES256),
    ) {
        Ok(v) => v,
        Err(e) => match e.kind() {
            // NOTE: This will fail if `exp` is specified as anything instead of a numeric or if a required field is empty
            jsonwebtoken::errors::ErrorKind::Json(e) => {
                let mut tags = Tags::default();
                tags.tags.insert(
                    "error".to_owned(),
                    match e.classify() {
                        serde_json::error::Category::Io => "IO_ERROR",
                        serde_json::error::Category::Syntax => "SYNTAX_ERROR",
                        serde_json::error::Category::Data => "DATA_ERROR",
                        serde_json::error::Category::Eof => "EOF_ERROR",
                    }
                    .to_owned(),
                );
                metrics
                    .clone()
                    .incr_with_tags("notification.auth.bad_vapid.json", Some(tags));
                if e.is_data() {
                    debug!("VAPID data warning: {:?}", e);
                    return Err(VapidError::InvalidVapid(
                        "A value in the vapid claims is either missing or incorrectly specified (e.g. \"exp\":\"12345\" or \"sub\":null). Please correct and retry.".to_owned(),
                    )
                    .into());
                }
                // Other errors are always possible. Try to be helpful by returning
                // the Json parse error.
                return Err(VapidError::InvalidVapid(e.to_string()).into());
            }
            _ => {
                let mut tags = Tags::default();
                tags.tags.insert("error".to_owned(), e.to_string());
                metrics
                    .clone()
                    .incr_with_tags("notification.auth.bad_vapid.other", Some(tags));
                error!("Bad Aud: Unexpected VAPID error: {:?}", &e);
                return Err(e.into());
            }
        },
    };

    // Dump the claims.
    // Note, this can produce a LOT of log messages if this feature is enabled.
    #[cfg(log_vapid)]
    if let Some(claims_str) = vapid.token.split('.').next() {
        use base64::Engine;
        info!(
            "Vapid";
            "sub" => &token_data.claims.sub,
            "claims" => String::from_utf8(
                base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(claims_str)
                    .unwrap_or_default()
            )
            .unwrap_or("UNKNOWN".to_owned())
        );
    };

    if token_data.claims.exp > (sec_since_epoch() + ONE_DAY_IN_SECONDS) {
        // The expiration is too far in the future
        return Err(VapidError::FutureExpirationToken.into());
    }

    let aud = match Url::from_str(&token_data.claims.aud) {
        Ok(v) => v,
        Err(_) => {
            error!("Bad Aud: Invalid audience {:?}", &token_data.claims.aud);
            metrics.clone().incr("notification.auth.bad_vapid.aud");
            return Err(VapidError::InvalidAudience.into());
        }
    };

    if domain != &aud {
        error!("Bad Aud: I am <{:?}>, asked for <{:?}> ", domain, aud);
        metrics.clone().incr("notification.auth.bad_vapid.domain");
        return Err(VapidError::InvalidAudience.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_vapid_jwt, VapidClaims};
    use crate::error::ApiErrorKind;
    use crate::extractors::subscription::repad_base64;
    use crate::headers::vapid::{VapidError, VapidHeader, VapidHeaderWithKey, VapidVersionData};
    use crate::metrics::Metrics;
    use autopush_common::util::{b64_decode_std, sec_since_epoch};
    use serde::{Deserialize, Serialize};
    use std::str::FromStr;
    use url::Url;

    #[test]
    fn repad_base64_1_padding() {
        assert_eq!(repad_base64("Zm9vYmE"), "Zm9vYmE=")
    }

    #[test]
    fn repad_base64_2_padding() {
        assert_eq!(repad_base64("Zm9vYg"), "Zm9vYg==")
    }

    #[test]
    fn vapid_aud_valid() {
        let priv_key = b64_decode_std(
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\
                    oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\
                    bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb",
        )
        .unwrap();
        // Specify a potentially invalid padding.
        let public_key = "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds==".to_owned();
        let domain = "https://push.services.mozilla.org";
        let jwk_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        let enc_key = jsonwebtoken::EncodingKey::from_ec_der(&priv_key);
        let claims = VapidClaims {
            exp: sec_since_epoch() + super::ONE_DAY_IN_SECONDS - 100,
            aud: domain.to_owned(),
            sub: "mailto:admin@example.com".to_owned(),
        };
        let token = jsonwebtoken::encode(&jwk_header, &claims, &enc_key).unwrap();

        let header = VapidHeaderWithKey {
            public_key,
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token,
                version_data: VapidVersionData::Version1,
            },
        };
        let result = validate_vapid_jwt(&header, &Url::from_str(domain).unwrap(), &Metrics::noop());
        assert!(result.is_ok());
    }

    #[test]
    fn vapid_aud_invalid() {
        let priv_key = b64_decode_std(
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\
                    oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\
                    bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb",
        )
        .unwrap();
        let public_key = "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds".to_owned();
        let domain = "https://push.services.mozilla.org";
        let jwk_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        let enc_key = jsonwebtoken::EncodingKey::from_ec_der(&priv_key);
        let claims = VapidClaims {
            exp: sec_since_epoch() + super::ONE_DAY_IN_SECONDS - 100,
            aud: domain.to_owned(),
            sub: "mailto:admin@example.com".to_owned(),
        };
        let token = jsonwebtoken::encode(&jwk_header, &claims, &enc_key).unwrap();
        let header = VapidHeaderWithKey {
            public_key,
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token,
                version_data: VapidVersionData::Version1,
            },
        };
        assert!(matches!(
            validate_vapid_jwt(
                &header,
                &Url::from_str("http://example.org").unwrap(),
                &Metrics::noop()
            )
            .unwrap_err()
            .kind,
            ApiErrorKind::VapidError(VapidError::InvalidAudience)
        ));
    }

    #[test]
    fn vapid_exp_is_string() {
        #[derive(Debug, Deserialize, Serialize)]
        struct StrExpVapidClaims {
            exp: String,
            aud: String,
            sub: String,
        }

        let priv_key = b64_decode_std(
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\
                    oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\
                    bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb",
        )
        .unwrap();
        let public_key = "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds".to_owned();
        let domain = "https://push.services.mozilla.org";
        let jwk_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        let enc_key = jsonwebtoken::EncodingKey::from_ec_der(&priv_key);
        let claims = StrExpVapidClaims {
            exp: (sec_since_epoch() + super::ONE_DAY_IN_SECONDS - 100).to_string(),
            aud: domain.to_owned(),
            sub: "mailto:admin@example.com".to_owned(),
        };
        let token = jsonwebtoken::encode(&jwk_header, &claims, &enc_key).unwrap();
        let header = VapidHeaderWithKey {
            public_key,
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token,
                version_data: VapidVersionData::Version1,
            },
        };
        let vv = validate_vapid_jwt(
            &header,
            &Url::from_str("http://example.org").unwrap(),
            &Metrics::noop(),
        )
        .unwrap_err()
        .kind;
        assert!(matches![
            vv,
            ApiErrorKind::VapidError(VapidError::InvalidVapid(_))
        ])
    }

    #[test]
    fn vapid_public_key_variants() {
        #[derive(Debug, Deserialize, Serialize)]
        struct StrExpVapidClaims {
            exp: String,
            aud: String,
            sub: String,
        }

        let priv_key = b64_decode_std(
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\
                    oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\
                    bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb",
        )
        .unwrap();
        // pretty much matches the kind of key we get from some partners.
        let public_key_standard = "BM3bVjW/wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf+1odb8hds=".to_owned();
        let public_key_url_safe = "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds=".to_owned();
        let domain = "https://push.services.mozilla.org";
        let jwk_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        let enc_key = jsonwebtoken::EncodingKey::from_ec_der(&priv_key);
        let claims = VapidClaims {
            exp: sec_since_epoch() + super::ONE_DAY_IN_SECONDS - 100,
            aud: domain.to_owned(),
            sub: "mailto:admin@example.com".to_owned(),
        };
        let token = jsonwebtoken::encode(&jwk_header, &claims, &enc_key).unwrap();
        // try standard form with padding
        let header = VapidHeaderWithKey {
            public_key: public_key_standard.clone(),
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token: token.clone(),
                version_data: VapidVersionData::Version1,
            },
        };
        assert!(
            validate_vapid_jwt(&header, &Url::from_str(domain).unwrap(), &Metrics::noop()).is_ok()
        );
        // try standard form with no padding
        let header = VapidHeaderWithKey {
            public_key: public_key_standard.trim_end_matches('=').to_owned(),
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token: token.clone(),
                version_data: VapidVersionData::Version1,
            },
        };
        assert!(
            validate_vapid_jwt(&header, &Url::from_str(domain).unwrap(), &Metrics::noop()).is_ok()
        );
        // try URL safe form with padding
        let header = VapidHeaderWithKey {
            public_key: public_key_url_safe.clone(),
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token: token.clone(),
                version_data: VapidVersionData::Version1,
            },
        };
        assert!(
            validate_vapid_jwt(&header, &Url::from_str(domain).unwrap(), &Metrics::noop()).is_ok()
        );
        // try URL safe form without padding
        let header = VapidHeaderWithKey {
            public_key: public_key_url_safe.trim_end_matches('=').to_owned(),
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token,
                version_data: VapidVersionData::Version1,
            },
        };
        assert!(
            validate_vapid_jwt(&header, &Url::from_str(domain).unwrap(), &Metrics::noop()).is_ok()
        );
    }

    #[test]
    fn vapid_missing_sub() {
        #[derive(Debug, Deserialize, Serialize)]
        struct NoSubVapidClaims {
            exp: u64,
            aud: String,
            sub: Option<String>,
        }

        let priv_key = b64_decode_std(
            "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\
                    oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\
                    bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb",
        )
        .unwrap();
        let public_key = "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds".to_owned();
        let domain = "https://push.services.mozilla.org";
        let jwk_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
        let enc_key = jsonwebtoken::EncodingKey::from_ec_der(&priv_key);
        let claims = NoSubVapidClaims {
            exp: sec_since_epoch() + super::ONE_DAY_IN_SECONDS - 100,
            aud: domain.to_owned(),
            sub: None,
        };
        let token = jsonwebtoken::encode(&jwk_header, &claims, &enc_key).unwrap();
        let header = VapidHeaderWithKey {
            public_key,
            vapid: VapidHeader {
                scheme: "vapid".to_string(),
                token,
                version_data: VapidVersionData::Version1,
            },
        };
        let vv = validate_vapid_jwt(
            &header,
            &Url::from_str("http://example.org").unwrap(),
            &Metrics::noop(),
        )
        .unwrap_err()
        .kind;
        assert!(matches![
            vv,
            ApiErrorKind::VapidError(VapidError::InvalidVapid(_))
        ])
    }
}
