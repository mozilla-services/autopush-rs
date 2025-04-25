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
    Utc::now().timestamp_micros() as u64
}

/// Display a formatted date-time string from a SystemTime
///
/// (This is useful in dev/debugging)
#[allow(dead_code)]
pub fn date_string_from_systemtime(ts: std::time::SystemTime) -> String {
    let dt: chrono::DateTime<chrono::Utc> = ts.into();
    dt.format("%Y-%m-%d %H:%M:%S.%f").to_string()
}

/// Display a formatted date-time string from a UTC offset in millis
///
/// (This is useful in dev/debugging)
#[allow(dead_code)]
pub fn date_string_from_utc_ms(offset: u64) -> String {
    let utc = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_millis(offset);
    date_string_from_systemtime(utc)
}
