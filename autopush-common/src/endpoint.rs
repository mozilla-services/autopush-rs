use crate::errors::ApiResult;
use base64::Engine;
use fernet::MultiFernet;
use openssl::hash;
use url::Url;
use uuid::Uuid;

/// Convenience wrapper for base64 decoding
/// *note* The `base64` devs are HIGHLY opinionated and the method to encode/decode
/// changes frequently. This function encapsulates that as much as possible.
pub fn b64_decode(input:&str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(input.trim_end_matches('='))
}

/// Create an v1 or v2 WebPush endpoint from the identifiers
///
/// Both endpoints use bytes instead of hex to reduce ID length.
//  v1 is the uaid + chid
//  v2 is the uaid + chid + sha256(key).bytes
pub fn make_endpoint(
    uaid: &Uuid,
    chid: &Uuid,
    key: Option<&str>,
    endpoint_url: &str,
    fernet: &MultiFernet,
) -> ApiResult<String> {
    let root = Url::parse(endpoint_url)?.join("wpush/")?;
    let mut base = uaid.as_bytes().to_vec();
    base.extend(chid.as_bytes());

    if let Some(k) = key {
        // *note*: Base64 devs are HIGHLY opinionated and tend to change things frequently.
        // expect that the following will chnage
        let raw_key = b64_decode(k)?;
        let key_digest = hash::hash(hash::MessageDigest::sha256(), &raw_key)?;
        base.extend(key_digest.iter());
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        let final_url = root.join(&format!("v2/{}", encrypted))?;
        Ok(final_url.to_string())
    } else {
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        let final_url = root.join(&format!("v1/{}", encrypted))?;
        Ok(final_url.to_string())
    }
}
