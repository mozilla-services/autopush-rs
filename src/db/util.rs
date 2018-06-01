use chrono::Utc;
use rand::{thread_rng, Rng};

/// Generate a last_connect
///
/// This intentionally generates a limited set of keys for each month in a
///  known sequence. For each month, there's 24 hours * 10 random numbers for
///  a total of 240 keys per month depending on when the user migrates forward.
pub fn generate_last_connect() -> u64 {
    let today = Utc::now();
    let mut rng = thread_rng();
    let num = rng.gen_range(0, 10);
    let val = format!("{}{:04}", today.format("%Y%m%H"), num);
    val.parse::<u64>().unwrap()
}
