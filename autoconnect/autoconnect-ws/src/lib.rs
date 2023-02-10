/// Handles ClientRegistry / max connections
/// ping handling
/// dispatch to ws-client-state_machine
use autopush_common::notification::Notification;

// Used for the server to flag a webpush client to deliver a Notification or Check storage
pub enum ServerNotification {
    CheckStorage,
    Notification(Notification),
    Disconnect,
}

impl Default for ServerNotification {
    fn default() -> Self {
        ServerNotification::Disconnect
    }
}
