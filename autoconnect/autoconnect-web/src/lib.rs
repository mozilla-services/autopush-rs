/// Handles most of the web interface aspects including
/// handling dockerflow endpoints, `/push/' and `/notif' endpoints
/// (used for inter-node routing)
/// Also used for MegaphoneUpdater and BroadcastChangeTracker endpoints
extern crate slog;
#[macro_use]
extern crate slog_scope;

pub mod broadcast;
pub mod client;
pub mod dockerflow;
pub mod metrics;
