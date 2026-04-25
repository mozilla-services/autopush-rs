use actix_web::{
    FromRequest,
    web::{Bytes, Data},
};
use autopush_common::util::b64_decode_url;
use futures::{FutureExt, future};
use openssl::hash::{self, DigestBytes};

use crate::{
    error::{ApiError, ApiErrorKind, ApiResult},
    extractors::forward_request::uncompressed_form,
    server::AppState,
};

pub struct ForwardToken(pub String);

/// HTTP body in JSON sent to [new_forward_endpoint]
#[derive(serde::Deserialize)]
pub struct ForwardEndpointRequest {
    /// URL to forward notifs to, must be HTTPS endpoint
    url: String,
    /// VAPID pubkey of incoming notifs, uncompressed format URL-safe Base64 encoded
    server_pubkey: Option<String>,
    /// VAPID privkey to outgoind notifs, PEM format - used only if server_pubkey is present
    forward_privkey: Option<String>,
}

impl FromRequest for ForwardToken {
    type Error = ApiError;
    type Future = future::LocalBoxFuture<'static, Result<Self, Self::Error>>;

    fn from_request(
        req: &actix_web::HttpRequest,
        payload: &mut actix_http::Payload,
    ) -> Self::Future {
        let req = req.clone();
        let mut payload = payload.take();
        async move {
            let app_state: Data<AppState> =
                Data::extract(&req).await.expect("No server state found");
            let data = Bytes::from_request(&req, &mut payload)
                .await
                .map_err(|e| ApiErrorKind::PayloadError(e))?;
            let req: ForwardEndpointRequest = serde_json::from_slice(&data)?;
            let root = url::Url::parse(&app_state.settings.endpoint_url).map_err(|e| {
                ApiErrorKind::General(format!("Cannot parse settings endpoint_url: {e}"))
            })?;

            let forward_url = url::Url::parse(&req.url)
                .map_err(|e| ApiErrorKind::General(format!("Cannot parse requested url: {e}")))?;

            check_forward_scheme(&forward_url)?;
            check_host(&root, &forward_url)?;

            let clear_token = make_cleartext_token(req)?;
            let token = app_state
                .fernet
                .encrypt(&clear_token)
                .trim_matches('=')
                .to_string();

            Ok(ForwardToken(token))
        }
        .boxed_local()
    }
}

/// Check that the forward URL is HTTPS
///
/// Debug builds also allow HTTP
fn check_forward_scheme(url: &url::Url) -> ApiResult<()> {
    // We allow http for debug build
    #[cfg(debug_assertions)]
    let invalid_scheme = url.scheme() != "https" && url.scheme() != "http";
    #[cfg(not(debug_assertions))]
    let invalid_scheme = url.scheme() != "https";

    if invalid_scheme {
        Err(ApiErrorKind::General(format!(
            "Invalid scheme {}",
            url.scheme()
        )))?
    }
    Ok(())
}

/// Make token from [ForwardEndpointRequest], must be fernet-encrypted
///
/// Contains: `{url: Vec<u8>}{pubkey_digest: [u8; 32]}{privkey_big_number: [u8; 32]}`
fn make_cleartext_token(req: ForwardEndpointRequest) -> ApiResult<Vec<u8>> {
    let mut clear_token = req.url.into_bytes();
    let server_pubkey_digest = if let Some(k) = req.server_pubkey {
        let digest = digest_from_pubkey(&k)?;
        clear_token.extend(digest.iter());
        Some(digest)
    } else {
        clear_token.extend([0u8; 32].iter());
        None
    };
    if let Some(der) = req.forward_privkey {
        let privkey = checked_privkey_from_der(&der, server_pubkey_digest)?;
        clear_token.extend(privkey.iter());
    } else {
        clear_token.extend([0u8; 32].iter());
    };
    Ok(clear_token)
}

/// Check that the forward URL isn't us
fn check_host(root: &url::Url, forward_url: &url::Url) -> ApiResult<()> {
    if forward_url.host() == root.host() {
        Err(ApiErrorKind::General(format!(
            "Cannot push to ourselves {}",
            forward_url.host_str().unwrap_or("None")
        )))?;
    }
    Ok(())
}

/// Get SHA256 digest of the pubkey
///
/// [pubkey] must be in the uncompress format, URL-safe Base64 encoded
pub fn digest_from_pubkey(pubkey: &str) -> ApiResult<hash::DigestBytes> {
    let raw_key = b64_decode_url(pubkey).map_err(|e| {
        warn!("Payload: error decoding user provided VAPID key:{:?}", e);
        ApiErrorKind::General("Error decoding VAPID key".to_owned())
    })?;
    let digest = hash::hash(hash::MessageDigest::sha256(), &raw_key).map_err(|e| {
        warn!("Payload: Error creating digest for VAPID key: {:?}", e);
        ApiErrorKind::General("Error creating message digest for key".to_owned())
    })?;
    Ok(digest)
}

/// Get private key big number, 32 bytes long Vec<u8>,
/// if the key is valid, and its public key isn't the server pubkey
///
/// [server_pubkey_digest]: SHA256 digest of the server_pubkey, uncompressed format
fn checked_privkey_from_der(
    der: &str,
    server_pubkey_digest: Option<DigestBytes>,
) -> ApiResult<Vec<u8>> {
    let server_pubkey_digest = server_pubkey_digest.ok_or_else(|| {
        ApiErrorKind::General(
        "The forward VAPID key can't be set without a server pubkey, used to get the VAPID claims."
            .into(),
    )
    })?;
    let eckey = openssl::ec::EcKey::private_key_from_pem(der.as_bytes()).map_err(|e| {
        warn!(
            "Payload: error decoding user provided outgoind VAPID key:{:?}",
            e
        );
        ApiErrorKind::General(format!(
            "Error decoding VAPID private key. It should be a valid PEM format: {e}"
        ))
    })?;

    eckey
        .check_key()
        .map_err(|e| ApiErrorKind::General(format!("Invalid VAPID private key: {e}")))?;

    let bytes_key = eckey
        .private_key()
        .to_vec_padded(32)
        .map_err(|e| ApiErrorKind::General(format!("Invalid VAPID private key length: {e}")))?;

    // Verify that the public key isn't for this private key's:
    // The private key must be a new one only to forward notifications!
    let uncompress_pubkey = uncompressed_form(eckey.public_key())
        .map_err(|_| ApiErrorKind::General("Error while checking public key".into()))?;

    let digest = hash::hash(hash::MessageDigest::sha256(), &uncompress_pubkey).map_err(|e| {
        warn!("Payload: Error creating digest for VAPID key: {:?}", e);
        ApiErrorKind::General("Error creating message digest for key".to_owned())
    })?;

    if openssl::memcmp::eq(&digest, &server_pubkey_digest) {
        Err(ApiErrorKind::General(
            "VAPID app_server public key must not be forward_key's public key".into(),
        ))?;
    }
    Ok(bytes_key)
}

#[cfg(test)]
mod test {
    use crate::extractors::forward_token::{
        ForwardEndpointRequest, check_forward_scheme, check_host, checked_privkey_from_der,
        digest_from_pubkey, make_cleartext_token,
    };

    pub const PRIV_KEY_DER: &str = "-----BEGIN PRIVATE KEY-----
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgZImOgpRszunnU3j1
oX5UQiX8KU4X2OdbENuvc/t8wpmhRANCAATN21Y1v8LmQueGpSG6o022gTbbYa4l
bXWZXITsjknW1WHmELtouYpyXX7e41FiAMuDvcRwW2Nfehn/taHW/IXb
-----END PRIVATE KEY-----";
    pub const PUB_KEY_FOR_PRIVATE: &str =
        "BM3bVjW_wuZC54alIbqjTbaBNtthriVtdZlchOyOSdbVYeYQu2i5inJdft7jUWIAy4O9xHBbY196Gf-1odb8hds";
    pub const ANOTHER_PUB_KEY: &str =
        "BA1Hxzyi1RUM1b5wjxsn7nGxAszw2u61m164i3MrAIxHF6YK5h4SDYic-dRuU_RcPcfA5aq9ojSwk5Y2EmclBPs";

    #[test]
    fn validate_scheme() {
        let url = url::Url::parse("unix:///run/test").expect("can't parse URL");

        check_forward_scheme(&url).expect_err("non-https scheme must not be accepted");
        let url = url::Url::parse("https://example.tld").expect("can't parse URL");
        check_forward_scheme(&url).expect("https scheme must be accepted");
    }

    #[test]
    fn validate_host() {
        let root = url::Url::parse("https:/update.example.tld").expect("can't parse URL");
        let url = url::Url::parse("https:/update.example.tld").expect("can't parse URL");

        check_host(&root, &url).expect_err("forward to our own host must be rejected");
        let url = url::Url::parse("https:/update2.example.tld").expect("can't parse URL");
        check_host(&root, &url).expect("forward to another host must be accepted");
    }

    #[test]
    fn extract_checked_bn() {
        checked_privkey_from_der(PRIV_KEY_DER, None).expect_err(
            "extracting privkey big number shouldn't be possible without a incoming VAPID pubkey",
        );

        let digest =
            digest_from_pubkey(PUB_KEY_FOR_PRIVATE).expect("can't extract digest from pubkey");
        checked_privkey_from_der(PRIV_KEY_DER, Some(digest)).expect_err("extracting privkey big number shouldn't be possible if incoming VAPID pubkey matches the forwaring private key");

        let digest = digest_from_pubkey(ANOTHER_PUB_KEY).expect("can't extract digest from pubkey");
        let bn = checked_privkey_from_der(PRIV_KEY_DER, Some(digest))
            .expect("can't extract privkey big number");
        assert_eq!(32, bn.len(), "extracted big number doesn't match length");
    }

    #[test]
    fn cleartext_token() {
        let req = ForwardEndpointRequest {
            url: "https://a/".into(),
            server_pubkey: None,
            forward_privkey: None,
        };
        let token = make_cleartext_token(req).expect("couldn't make token from request");
        assert_eq!(74, token.len(), "token length mismatch");
        assert_eq!(
            [0u8; 32],
            token[10..42],
            "digest section mismatch without pubkey"
        );
        assert_eq!(
            [0u8; 32],
            token[42..74],
            "digest section mismatch without privkey der"
        );

        let req = ForwardEndpointRequest {
            url: "https://a/".into(),
            server_pubkey: Some(ANOTHER_PUB_KEY.to_string()),
            forward_privkey: None,
        };
        let token = make_cleartext_token(req).expect("couldn't make token from request");
        assert_eq!(74, token.len(), "token length mismatch");
        assert_ne!(
            [0u8; 32],
            token[10..42],
            "digest section mismatch with pubkey"
        );
        assert_eq!(
            [0u8; 32],
            token[42..74],
            "digest section mismatch without privkey der"
        );

        let req = ForwardEndpointRequest {
            url: "https://a/".into(),
            server_pubkey: Some(ANOTHER_PUB_KEY.to_string()),
            forward_privkey: Some(PRIV_KEY_DER.to_string()),
        };
        let token = make_cleartext_token(req).expect("couldn't make token from request");
        assert_eq!(74, token.len(), "token length mismatch");
        assert_ne!(
            [0u8; 32],
            token[10..42],
            "digest section mismatch with pubkey"
        );
        assert_ne!(
            [0u8; 32],
            token[42..74],
            "digest section mismatch with privkey der"
        );
    }
}
