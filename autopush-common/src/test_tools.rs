use uuid::Uuid;

/// Generate a UAID that is prefixed with the test-identification ID "DEADBEEF".
/// Note: It's absolutely possible that this might cause a conflict with valid UAIDs, but
/// the risk is reasonably small, and we could limit pruning to whenever we had
/// accidentally run the test script against production.
pub fn gen_test_uaid() -> Uuid {
    let temp = Uuid::new_v4().to_string();
    let mut parts: Vec<&str> = temp.split('-').collect();
    parts[0] = "DEADBEEF";
    Uuid::parse_str(&parts.join("-")).unwrap()
}
