use crate::errors::{ApcErrorKind, Result};
use crate::util::b64_decode_url;

use fernet::MultiFernet;
use openssl::hash;
use url::Url;
use uuid::Uuid;

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
) -> Result<String> {
    let root = Url::parse(endpoint_url)?.join("wpush/")?;
    let mut base = uaid.as_bytes().to_vec();
    base.extend(chid.as_bytes());

    if let Some(k) = key {
        let raw_key = b64_decode_url(k)
            .map_err(|_e| ApcErrorKind::PayloadError("Error encrypting payload".to_owned()))?;
        let key_digest = hash::hash(hash::MessageDigest::sha256(), &raw_key).map_err(|_e| {
            ApcErrorKind::PayloadError("Error creating message digest for key".to_owned())
        })?;
        base.extend(key_digest.iter());
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        let final_url = root.join(&format!("v2/{encrypted}")).map_err(|_e| {
            ApcErrorKind::PayloadError("Encrypted data is not URL-safe".to_owned())
        })?;
        Ok(final_url.to_string())
    } else {
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        let final_url = root.join(&format!("v1/{encrypted}")).map_err(|_e| {
            ApcErrorKind::PayloadError("Encrypted data is not URL-safe".to_owned())
        })?;
        Ok(final_url.to_string())
    }
}
