/// Extracts notification data from `Subscription` and request data
pub struct Notification {
    pub channel_id: String,
    pub version: String,
    pub ttl: String,
    pub topic: String,
    pub timestamp: String,
    pub data: String,
    pub headers: String,
}
