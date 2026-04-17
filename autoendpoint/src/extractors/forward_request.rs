use std::str::FromStr;

use actix_http::header::{self, HeaderMap};
use actix_web::{FromRequest, HttpRequest, web::Data};
use autopush_common::{
    metric_name::MetricName, metrics::StatsdClientExt as _, util::b64_encode_url,
};
use cadence::StatsdClient;
use futures::{FutureExt, future};
use openssl::{ec::PointConversionForm, hash::MessageDigest};

use crate::{
    error::{ApiError, ApiErrorKind, ApiResult},
    extractors::subscription::{decode_public_key, repad_base64, validate_vapid_jwt},
    headers::vapid::{VapidClaims, VapidError, VapidHeader, VapidHeaderWithKey, VapidVersionData},
    server::AppState,
    settings::Settings,
};

/// Contains data extracted from the encrypted token and the request, required to push to the new endpoint
#[derive(Debug)]
struct ForwardData {
    /// Endpoint to forward notifications
    url: String,
    /// VAPID claims from the incoming authorization header, reused for outgoing request
    vapid_claims: Option<VapidClaims>,
    /// VAPID private key for outgoing request
    vapid_privkey: Option<[u8; 32]>,
}

/// Contains the [reqwest::RequestBuilder] made from data extracted from the encrypted token
/// via [ForwardToken]
pub struct ForwardRequest(pub reqwest::RequestBuilder);

impl FromRequest for ForwardData {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_http::Payload) -> Self::Future {
        let req = req.clone();
        async move {
            let app_state: Data<AppState> =
                Data::extract(&req).await.expect("No server state found");

            let encoded_token = req
                .match_info()
                .get("token")
                .expect("{token} must be part of the webpush path")
                .to_string();

            let raw_token = app_state
                .fernet
                .decrypt(&repad_base64(&encoded_token))
                .map_err(|e| {
                    // Since we're decrypting and endpoint, we get a lot of spam links.
                    // This can fill our logs.
                    trace!("🔐 fernet: {:?}", e.to_string());
                    ApiErrorKind::InvalidToken
                })?;
            forward_data_from_token(
                raw_token,
                &req.headers(),
                &app_state.settings,
                &app_state.metrics,
            )
        }
        .boxed_local()
    }
}

/// Get the [ForwardData] from the token and the request headers
fn forward_data_from_token(
    token: Vec<u8>,
    req_headers: &HeaderMap,
    settings: &Settings,
    metrics: &StatsdClient,
) -> ApiResult<ForwardData> {
    if token.len() < 64 {
        trace!("⏩️forward: token shorter than 64 bytes");
        Err(ApiErrorKind::InvalidToken)?;
    }
    let i_pubkey = token.len() - 64;
    let i_privkey = i_pubkey + 32;

    let url = String::from_utf8(token[..i_pubkey].to_vec())
        .ok()
        .map(|s| url::Url::parse(&s).ok())
        .flatten()
        .ok_or_else(|| ApiErrorKind::InvalidToken)?;

    let pubkey_hash: Option<[u8; 32]> = if token[i_pubkey..i_privkey] == [0u8; 32] {
        None
    } else {
        token[i_pubkey..i_privkey].try_into().ok()
    };

    let vapid_privkey: Option<[u8; 32]> = if token[i_privkey..] == [0u8; 32] {
        None
    } else {
        token[i_privkey..].try_into().ok()
    };

    trace!(
        "⏩️forward: new request to {}, inbound vapid {}, outboud vapid {}",
        url,
        pubkey_hash.is_some(),
        vapid_privkey.is_some()
    );

    let vapid_claims = get_vapid_claims(
        req_headers,
        pubkey_hash,
        url.origin().unicode_serialization(),
        settings,
        metrics,
    )?;

    Ok(ForwardData {
        url: url.into(),
        vapid_claims,
        vapid_privkey,
    })
}

/// Get VAPID claims, AUD edited for the forwarded url, if the token is configured to use VAPID
///
/// Fails if the token is configured to use VAPID, and the authorization doesn't succeed
fn get_vapid_claims(
    headers: &HeaderMap,
    pubkey_hash: Option<[u8; 32]>,
    forward_origin: String,
    settings: &Settings,
    metrics: &StatsdClient,
) -> ApiResult<Option<VapidClaims>> {
    if let Some(token_pubkey) = pubkey_hash {
        match headers
            .get("authorization")
            .map(|h| h.to_str().ok())
            .flatten()
        {
            None => {
                let e = VapidError::MissingKey;
                metrics
                    .incr_with_tags(MetricName::NotificationAuthError)
                    .with_tag("error", e.as_metric())
                    .send();
                Err(e)?
            }
            Some(auth_header) => {
                let vapid = VapidHeader::parse(auth_header).inspect_err(|e| {
                    metrics
                        .incr_with_tags(MetricName::NotificationAuthError)
                        .with_tag("error", e.as_metric())
                        .send();
                })?;
                // We do not support old VAPID draft
                let vapid = match &vapid.version_data {
                    VapidVersionData::Version1 => Err(ApiErrorKind::InvalidAuthentication),
                    VapidVersionData::Version2 { public_key } => {
                        let public_key = public_key.clone();
                        Ok(VapidHeaderWithKey { vapid, public_key })
                    }
                }?;
                let public_key = decode_public_key(&vapid.public_key)?;
                let key_hash = openssl::hash::hash(MessageDigest::sha256(), &public_key)
                    .map_err(ApiErrorKind::TokenHashValidation)?;
                if !openssl::memcmp::eq(&key_hash, &token_pubkey) {
                    return Err(VapidError::KeyMismatch.into());
                }
                let mut claims = validate_vapid_jwt(&vapid, settings, metrics)?;
                claims.aud = Some(forward_origin);
                Ok(Some(claims))
            }
        }
    } else {
        Ok(None)
    }
}

impl FromRequest for ForwardRequest {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_http::Payload) -> Self::Future {
        let req = req.clone();
        async move {
            let mut in_headers = req.headers().clone();
            check_content_encoding(&in_headers)?;

            let app_state: Data<AppState> =
                Data::extract(&req).await.expect("No server state found");
            let data = ForwardData::extract(&req).await?;

            if let Some(vapid_privkey) = data.vapid_privkey {
                set_vapid_authorization(&mut in_headers, &data.vapid_claims, &vapid_privkey)?;
            } else {
                in_headers.remove(header::AUTHORIZATION);
            }
            in_headers.remove(header::HOST);
            let req_headers = actix_to_reqwest_headers(in_headers);

            // Note: we should not follow redirect. reqwest can't handle per-request
            // redirect policy atm (https://github.com/seanmonstar/reqwest/issues/353).
            // app_state.http is used only for webpush routers (which requests autopush nodes),
            // we can probably change its redirect policy. Else we can create a new
            // app_state.http_no_redir client
            Ok(Self(app_state.http.post(data.url).headers(req_headers)))
        }
        .boxed_local()
    }
}

fn check_content_encoding(in_headers: &HeaderMap) -> ApiResult<()> {
    let encoding = in_headers
        .get("Content-Encoding")
        .ok_or_else(|| ApiErrorKind::InvalidEncryption("Missing Content-Encoding header".into()))?
        .to_str()
        .map_err(|_| ApiErrorKind::InvalidEncryption("Invalid Content-Encoding header".into()))?;
    match encoding {
        "aes128gcm" => Ok(()),
        "aesgcm" => Err(ApiErrorKind::InvalidEncryption(
            "Unsupported legacy encryption".into(),
        )),
        _ => Err(ApiErrorKind::InvalidEncryption(
            "Unknown Content-Encoding header".into(),
        )),
    }?;
    Ok(())
}

fn set_vapid_authorization(
    in_headers: &mut HeaderMap,
    claims: &Option<VapidClaims>,
    vapid_privkey: &[u8; 32],
) -> ApiResult<()> {
    let key = private_bn_to_key_for_header(vapid_privkey)
        .map_err(|_| ApiErrorKind::General("Invalid private key".into()))?;

    let jwk_header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::ES256);
    let enc_key = jsonwebtoken::EncodingKey::from_ec_der(&key.pkcs8_der);
    let jwt = jsonwebtoken::encode(&jwk_header, claims, &enc_key)
        .map_err(|_| ApiErrorKind::General("Error while signing JWT".into()))?;
    in_headers.insert(
        header::AUTHORIZATION,
        format!("vapid t={},k={}", jwt, b64_encode_url(&key.public_fp))
            .try_into()
            .map_err(|_| ApiErrorKind::General("Couldn't create VAPID header value".into()))?,
    );
    Ok(())
}

fn actix_to_reqwest_headers(in_headers: HeaderMap) -> reqwest::header::HeaderMap {
    let mut req_headers = reqwest::header::HeaderMap::new();
    for (name, value) in in_headers {
        if let Ok(name) = reqwest::header::HeaderName::from_str(name.as_str()) {
            if let Ok(value) = value.as_bytes().try_into() {
                req_headers.append(name, value);
            } else {
                trace!("⏩️forward: received invalid header value {value:?}");
            }
        } else {
            trace!("⏩️forward: received invalid header {name}");
        }
    }
    req_headers
}

struct VapidPrivateForHeader {
    pkcs8_der: Vec<u8>,
    public_fp: Vec<u8>,
}

fn private_bn_to_key_for_header(
    vapid_privkey: &[u8],
) -> Result<VapidPrivateForHeader, Box<dyn std::error::Error>> {
    let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::X9_62_PRIME256V1)?;
    let private_number = openssl::bn::BigNum::from_slice(vapid_privkey)?;
    let mut ctx = openssl::bn::BigNumContext::new_secure()?;
    let mut pub_point = openssl::ec::EcPoint::new(&group)?;
    pub_point.mul_generator(&group, &private_number, &mut ctx)?;
    let key = openssl::ec::EcKey::from_private_components(&group, &private_number, &pub_point)?;
    let public_fp = uncompressed_form(key.public_key())?;
    let pkey = openssl::pkey::PKey::from_ec_key(key)?;
    Ok(VapidPrivateForHeader {
        pkcs8_der: pkey.private_key_to_pkcs8()?,
        public_fp,
    })
}

pub fn uncompressed_form(
    pubkey: &openssl::ec::EcPointRef,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let group = openssl::ec::EcGroup::from_curve_name(openssl::nid::Nid::X9_62_PRIME256V1)?;
    let mut ctx = openssl::bn::BigNumContext::new()?;
    Ok(pubkey.to_bytes(&group, PointConversionForm::UNCOMPRESSED, &mut ctx)?)
}

#[cfg(test)]
mod test {
    use actix_http::header::{HeaderMap, HeaderName, HeaderValue};
    use autopush_common::util::{b64_decode_url, b64_encode_std, b64_encode_url};
    use lazy_static::lazy_static;

    use crate::extractors::forward_request::forward_data_from_token;
    use crate::metrics::Metrics;
    use crate::{
        extractors::{
            forward_request::{
                check_content_encoding, get_vapid_claims, private_bn_to_key_for_header,
                set_vapid_authorization,
            },
            forward_token::digest_from_pubkey,
        },
        headers::vapid::VapidClaims,
        settings::Settings,
    };

    pub const OUT_PRIV_KEY_DER: &str = "\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1\
oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l\
bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb";
    pub const OUT_PUB_KEY: &str =
        "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds";
    pub const IN_PUB_KEY: &str =
        "BAZOxJhHcrJ6DVhRzeccitRqBWEUkOUFJAFbpxR4d2QcKscpKd2OvvmIWof3R7SWgsFpmO9e0vTukAYTY6nQiY0";

    lazy_static! {
        static ref OUT_PRIVKEY_BN: [u8; 32] =
            b64_decode_url("ZImOgpRszunnU3j1oX5UQiX8KU4X2OdbENuvc_t8wpk")
                .expect("can't init PRIVKEY_BN")
                .try_into()
                .expect("can't cast PRIVKEY_BN");
        /// This is used to generate incoming VAPID header for the test, this should normaly
        /// never be known to us
        static ref IN_PRIVKEY_BN: [u8; 32] =
            b64_decode_url("avIkQ9fhUl9tds7FxVtMEioXCv6ffFdw_7UGIR8JXUE")
                .expect("can't init PRIVKEY_BN")
                .try_into()
                .expect("can't cast PRIVKEY_BN");
    }

    #[test]
    fn content_encoding() {
        check_content_encoding(&HeaderMap::new())
            .expect_err("request without content-encoding must be rejected");

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-encoding"),
            HeaderValue::from_static("aesgcm"),
        );
        headers.insert(
            HeaderName::from_static("encryption"),
            HeaderValue::from_static("salt=foo"),
        );
        headers.insert(
            HeaderName::from_static("crypto-key"),
            HeaderValue::from_static("dh=bar"),
        );
        check_content_encoding(&headers)
            .expect_err("request with draft content-encoding must be rejected");

        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_static("content-encoding"),
            HeaderValue::from_static("aes128gcm"),
        );
        check_content_encoding(&headers)
            .expect("request with standard content-encoding must be accepted");
    }

    #[test]
    fn bn_to_key_for_header() {
        let key = private_bn_to_key_for_header(OUT_PRIVKEY_BN.as_ref())
            .expect("couldn't extract key from big number");
        assert_eq!(
            "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds",
            b64_encode_url(&key.public_fp),
            "public key mismatch"
        );
        assert_eq!(
            OUT_PRIV_KEY_DER,
            b64_encode_std(&key.pkcs8_der),
            "private key DER mismatch"
        );
    }

    #[test]
    fn insert_vapid() {
        let mut headers = HeaderMap::new();
        let claims = VapidClaims {
            aud: Some("https://target.example.tld".to_string()),
            ..Default::default()
        };
        set_vapid_authorization(&mut headers, &Some(claims), &OUT_PRIVKEY_BN)
            .expect("can't set vapid auth");
        let auth = headers
            .get("authorization")
            .expect("authorization header not set");
        let auth = auth.to_str().expect("can't extract string from auth");
        assert!(
            auth.starts_with("vapid t="),
            "auth header doesn't start with `vapid t=`"
        );
        assert!(
            auth.ends_with(&format!(",k={}", OUT_PUB_KEY)),
            "auth header doesn't end with `,k=PUBKEY`"
        );
    }

    #[test]
    fn extracted_vapid_claims() {
        let mut headers = HeaderMap::new();
        let our_origin = "https://us.example.tld".to_string();
        let forward_origin = "https://forward.example.tld".to_string();
        let claims = VapidClaims {
            aud: Some(our_origin.clone()),
            ..Default::default()
        };
        let test_settings = Settings {
            endpoint_url: our_origin,
            ..Default::default()
        };
        set_vapid_authorization(&mut headers, &Some(claims), &OUT_PRIVKEY_BN)
            .expect("can't set vapid auth");
        // digest is the SHA256 hash of the pubkey of the key that signed the VAPID header
        let digest = digest_from_pubkey(OUT_PUB_KEY)
            .expect("can't get digest from pubkey")
            .to_vec()
            .try_into()
            .expect("Can't cast DigestBytes to [u8; 32]");

        let claims = get_vapid_claims(
            &headers,
            Some(digest),
            forward_origin.clone(),
            &test_settings,
            &Metrics::sink(),
        )
        .expect("can't extract claims from headers with authorization")
        .expect("no claims found, when authorization is set");

        assert_eq!(
            Some(&forward_origin),
            claims.aud.as_ref(),
            "VAPID aud claim not correctly replaced",
        );

        get_vapid_claims(
            &HeaderMap::new(),
            Some(digest),
            forward_origin.clone(),
            &test_settings,
            &Metrics::sink(),
        )
        .expect_err("headers without vapid must fail if a pubkey digest is set");

        let claims = get_vapid_claims(
            &HeaderMap::new(),
            None,
            forward_origin,
            &test_settings,
            &Metrics::sink(),
        )
        .expect("can't extract claims from headers without authorization");
        assert_eq!(
            None, claims,
            "no claims must be exported without authorization",
        );
    }

    #[test]
    fn forward_data_no_auth() {
        let forward_url = "https://forward.domain.tld/abcd".to_string();
        let test_settings = Settings {
            endpoint_url: "https://us.domain.tld".to_string(),
            ..Default::default()
        };

        let mut token = Vec::new();
        token.extend_from_slice(forward_url.as_bytes());
        token.extend_from_slice(&[0u8; 32]);
        token.extend_from_slice(&[0u8; 32]);
        let data =
            forward_data_from_token(token, &HeaderMap::new(), &test_settings, &Metrics::sink())
                .expect("can't extract data from token+headers without authentication");
        assert_eq!(forward_url, data.url, "forward origin doesn't match");
        assert_eq!(
            None, data.vapid_claims,
            "data must not have claims without authentication"
        );
        assert_eq!(None, data.vapid_privkey, "data must not have a private key");
    }

    #[test]
    fn forward_data_auth_valid() {
        let forward_url = "https://forward.domain.tld/abcd".to_string();
        let our_origin = "https://us.domain.tld".to_string();
        let test_settings = Settings {
            endpoint_url: our_origin.clone(),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        let claims = VapidClaims {
            aud: Some(our_origin.clone()),
            ..Default::default()
        };
        set_vapid_authorization(&mut headers, &Some(claims.clone()), &IN_PRIVKEY_BN)
            .expect("can't init in_headers");

        // without forward key
        let mut token = Vec::new();
        token.extend_from_slice(forward_url.as_bytes());
        let digest = digest_from_pubkey(&IN_PUB_KEY).expect("cann't get digest from pubkey");
        token.extend(digest.iter());
        token.extend_from_slice(&[0u8; 32]);
        let data = forward_data_from_token(token, &headers, &test_settings, &Metrics::sink())
            .expect("can't extract data from token+headers with authentication");
        assert_eq!(forward_url, data.url, "forward origin doesn't match");
        assert_eq!(
            data.vapid_claims.map(|c| c.aud).flatten(),
            Some(
                url::Url::parse(&forward_url)
                    .expect("can't parse forward url")
                    .origin()
                    .unicode_serialization()
            ),
            "data claim doesn't have the forward_url audience"
        );
        assert_eq!(None, data.vapid_privkey, "data must not have a private key");

        // with forward key
        let mut token = Vec::new();
        token.extend_from_slice(forward_url.as_bytes());
        let digest = digest_from_pubkey(&IN_PUB_KEY).expect("cann't get digest from pubkey");
        token.extend(digest.iter());
        token.extend_from_slice(OUT_PRIVKEY_BN.as_ref());
        let data = forward_data_from_token(token, &headers, &test_settings, &Metrics::sink())
            .expect("can't extract data from token+headers with authentication");
        assert_eq!(forward_url, data.url, "forward origin doesn't match");
        assert_eq!(
            data.vapid_claims.map(|c| c.aud).flatten(),
            Some(
                url::Url::parse(&forward_url)
                    .expect("can't parse forward url")
                    .origin()
                    .unicode_serialization()
            ),
            "data claim doesn't have the forward_url audience"
        );
        assert_eq!(
            Some(OUT_PRIVKEY_BN.to_vec()),
            data.vapid_privkey.map(|v| v.to_vec()),
            "private key doesn't match"
        );
    }

    #[test]
    fn forward_data_auth_invalid() {
        let forward_url = "https://forward.domain.tld/abcd".to_string();
        let our_origin = "https://us.domain.tld".to_string();
        let test_settings = Settings {
            endpoint_url: our_origin.clone(),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        let claims = VapidClaims {
            aud: Some(our_origin.clone()),
            ..Default::default()
        };
        set_vapid_authorization(&mut headers, &Some(claims.clone()), &IN_PRIVKEY_BN)
            .expect("can't init in_headers");

        let mut token = Vec::new();
        token.extend_from_slice(forward_url.as_bytes());
        let digest = digest_from_pubkey(&IN_PUB_KEY).expect("cann't get digest from pubkey");
        token.extend(digest.iter());
        token.extend_from_slice(&[0u8; 32]);
        forward_data_from_token(token, &HeaderMap::new(), &test_settings, &Metrics::sink())
                .expect_err("extract data must not be possible when token has pubkey, but request doesn't have authorization");
    }
}
