/// Handles ClientRegistry / max connections
/// ping handling
/// dispatch to ws-client-state_machine

// Used for the server to flag a webpush client to deliver a Notification or Check storage
#[derive(Clone, Debug)]
pub enum ServerNotification {
    CheckStorage,
    Notification(autopush_common::notification::Notification),
    Disconnect,
}

impl Default for ServerNotification {
    fn default() -> Self {
        ServerNotification::Disconnect
    }
}
