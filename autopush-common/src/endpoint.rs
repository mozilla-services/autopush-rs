use crate::errors::{Result, ResultExt};
use fernet::MultiFernet;
use openssl::hash;
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
    let root = format!("{}/wpush/", endpoint_url);
    let mut base = hex::decode(uaid.to_simple().to_string()).chain_err(|| "Error decoding")?;
    base.extend(hex::decode(chid.to_simple().to_string()).chain_err(|| "Error decoding")?);

    if let Some(k) = key {
        let raw_key =
            base64::decode_config(k, base64::URL_SAFE).chain_err(|| "Error encrypting payload")?;
        let key_digest = hash::hash(hash::MessageDigest::sha256(), &raw_key)
            .chain_err(|| "Error creating message digest for key")?;
        base.extend(key_digest.iter());
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        Ok(format!("{}v2/{}", root, encrypted))
    } else {
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        Ok(format!("{}v1/{}", root, encrypted))
    }
}
