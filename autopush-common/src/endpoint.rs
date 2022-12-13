use crate::errors::{Result, ResultExt};
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
    let root = Url::parse(endpoint_url)
        .chain_err(|| "endpoint_url is not a valid URL")?
        .join("wpush/")
        .chain_err(|| "Error creating URL")?;
    let mut base = uaid.as_bytes().to_vec();
    base.extend(chid.as_bytes());

    if let Some(k) = key {
        let raw_key = base64::decode_engine(
            k.trim_end_matches('='),
            &base64::engine::fast_portable::FastPortable::from(
                &base64::alphabet::URL_SAFE,
                base64::engine::fast_portable::NO_PAD,
            ),
        )
        .chain_err(|| "Error encrypting payload")?;
        let key_digest = hash::hash(hash::MessageDigest::sha256(), &raw_key)
            .chain_err(|| "Error creating message digest for key")?;
        base.extend(key_digest.iter());
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        let final_url = root
            .join(&format!("v2/{}", encrypted))
            .chain_err(|| "Encrypted data is not URL-safe")?;
        Ok(final_url.to_string())
    } else {
        let encrypted = fernet.encrypt(&base).trim_matches('=').to_string();
        let final_url = root
            .join(&format!("v1/{}", encrypted))
            .chain_err(|| "Encrypted data is not URL-safe")?;
        Ok(final_url.to_string())
    }
}
