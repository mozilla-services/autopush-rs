use chrono::prelude::*;

/// Get the time since the UNIX epoch in seconds
pub fn sec_since_epoch() -> u64 {
    Utc::now().timestamp() as u64
}

/// Get the time since the UNIX epoch in milliseconds
pub fn ms_since_epoch() -> u64 {
    Utc::now().timestamp_millis() as u64
}

/// Return UTC midnight for the current day.
pub fn ms_utc_midnight() -> u64 {
    let now = Utc::now();
    Utc.with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
        .single()
        .expect("Clock error") // Something had to go horribly wrong. Panic is ok.
        .timestamp_millis() as u64
}

/// Get the time since the UNIX epoch in microseconds
#[allow(dead_code)]
pub fn us_since_epoch() -> u64 {
    let now = Utc::now();
    (now.timestamp() as u64) * 1_000_000 + (now.timestamp_subsec_micros() as u64)
}
