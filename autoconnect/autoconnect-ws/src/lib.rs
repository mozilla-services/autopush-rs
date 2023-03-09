/// Handles ClientRegistry / max connections
/// ping handling
/// dispatch to ws-client-state_machine

// Used for the server to flag a webpush client to deliver a Notification or Check storage
#[derive(Clone, Debug, Default)]
pub enum ServerNotification {
    CheckStorage,
    Notification(autopush_common::notification::Notification),
    #[default]
    Disconnect,
}

