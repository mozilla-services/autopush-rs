use chrono::prelude::*;

/// Get the time since the UNIX epoch in seconds
pub fn sec_since_epoch() -> u64 {
    Utc::now().timestamp() as u64
}

/// Get the time since the UNIX epoch in milliseconds
pub fn ms_since_epoch() -> u64 {
    Utc::now().timestamp_millis() as u64
}

/// Get the time since the UNIX epoch in microseconds
#[allow(dead_code)]
pub fn us_since_epoch() -> u64 {
    let now = Utc::now();
    (now.timestamp() as u64) * 1_000_000 + (now.timestamp_subsec_micros() as u64)
}
