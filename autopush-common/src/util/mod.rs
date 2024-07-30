//! Various small utilities accumulated over time for the WebPush server
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Deserializer};

pub mod timing;
pub mod user_agent;

pub use self::timing::{ms_since_epoch, ms_utc_midnight, sec_since_epoch, us_since_epoch};

pub const ONE_DAY_IN_SECONDS: u64 = 60 * 60 * 24;

pub trait InsertOpt<K: Eq + Hash, V> {
    /// Insert an item only if it exists
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>);
}

impl<K: Eq + Hash, V> InsertOpt<K, V> for HashMap<K, V> {
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>) {
        if let Some(value) = value {
            self.insert(key.into(), value.into());
        }
    }
}

/// Convenience wrapper for base64 decoding
/// *note* The `base64` devs are HIGHLY opinionated and the method to encode/decode
/// changes frequently. This function encapsulates that as much as possible.
pub fn b64_decode_url(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(input.trim_end_matches('='))
}

pub fn b64_encode_url(input: &Vec<u8>) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input)
}

pub fn b64_decode_std(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    base64::engine::general_purpose::STANDARD_NO_PAD.decode(input.trim_end_matches('='))
}

pub fn b64_encode_std(input: &Vec<u8>) -> String {
    base64::engine::general_purpose::STANDARD_NO_PAD.encode(input)
}

pub fn deserialize_u32_to_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let seconds: u32 = Deserialize::deserialize(deserializer)?;
    Ok(Duration::from_secs(seconds.into()))
}

pub fn deserialize_opt_u32_to_duration<'de, D>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let seconds: Option<u32> = Deserialize::deserialize(deserializer)?;
    Ok(seconds.map(|v| Duration::from_secs(v.into())))
}
