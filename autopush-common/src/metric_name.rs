//! Defines standard metric names used across the application.
//!
//! This module provides a type-safe way to refer to metrics by replacing
//! string literals with enum variants, ensuring consistency and discoverability.

use strum::{AsRefStr, Display, EnumString};
use strum_macros::IntoStaticStr;

/// Represents all metric names used in the application.
#[derive(Debug, Clone, IntoStaticStr, AsRefStr, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum MetricName {
    /// User agent command with dynamic command name
    #[strum(serialize = "ua.command")]
    UaCommand(String),
    //
    // User agent metrics
    //
    /// User agent is already connected
    #[strum(serialize = "ua.already_connected")]
    UaAlreadyConnected,

    /// User agent register command
    #[strum(serialize = "ua.command.register")]
    UaCommandRegister,

    /// User agent unregister command
    #[strum(serialize = "ua.command.unregister")]
    UaCommandUnregister,

    /// User agent connection check
    #[strum(serialize = "ua.connection.check")]
    UaConnectionCheck,

    /// User agent connection channel count
    #[strum(serialize = "ua.connection.channel_count")]
    UaConnectionChannelCount,

    /// User agent notification sent
    #[strum(serialize = "ua.notification.sent")]
    UaNotificationSent,

    /// User agent expiration
    #[strum(serialize = "ua.expiration")]
    UaExpiration,

    //
    // Notification metrics
    //
    /// Notification authentication
    #[strum(serialize = "notification.auth")]
    NotificationAuth,

    /// Notification authentication bad VAPID JSON
    #[strum(serialize = "notification.auth.bad_vapid.json")]
    NotificationAuthBadVapidJson,

    /// Notification authentication bad VAPID other
    #[strum(serialize = "notification.auth.bad_vapid.other")]
    NotificationAuthBadVapidOther,

    /// Authentication success
    #[strum(serialize = "notification.auth.ok")]
    NotificationAuthOk,

    /// Authentication error
    #[strum(serialize = "notification.auth.error")]
    NotificationAuthError,

    /// Notification message expired
    #[strum(serialize = "notification.message.expired")]
    NotificationMessageExpired,

    /// Bridge error in notification routing
    #[strum(serialize = "notification.bridge.error")]
    NotificationBridgeError,

    /// Bridge successfully sent notification
    #[strum(serialize = "notification.bridge.sent")]
    NotificationBridgeSent,

    /// Notification total request time
    #[strum(serialize = "notification.total_request_time")]
    NotificationTotalRequestTime,

    /// Notification message data
    #[strum(serialize = "notification.message_data")]
    NotificationMessageData,

    /// Notifcation message stored
    #[strum(serialize = "notification.message.stored")]
    NotificationMessageStored,

    /// Notifcation message deleted
    #[strum(serialize = "notification.message.deleted")]
    NotificationMessageDeleted,

    //
    // Error metrics
    //
    /// Node timeout error
    #[strum(serialize = "error.node.timeout")]
    ErrorNodeTimeout,

    /// Node connection error
    #[strum(serialize = "error.node.connect")]
    ErrorNodeConnect,

    //
    // Update metrics
    //
    /// Updates drop user
    #[strum(serialize = "updates.drop_user")]
    UpdatesDropUser,

    /// VAPID version updates
    #[strum(serialize = "updates.vapid.draft")]
    UpdatesVapidDraft,

    /// Client host is gone
    #[strum(serialize = "updates.client.host_gone")]
    UpdatesClientHostGone,

    /// VAPID update
    #[strum(serialize = "updates.vapid")]
    UpdatesVapid,

    //
    // Megaphone metrics
    //
    /// Megaphone updater successful
    #[strum(serialize = "megaphone.updater.ok")]
    MegaphoneUpdaterOk,

    /// Megaphone updater error
    #[strum(serialize = "megaphone.updater.error")]
    MegaphoneUpdaterError,

    //
    // Reliability metrics
    //
    /// Reliability error with Redis unavailable
    #[strum(serialize = "reliability.error.redis_unavailable")]
    ReliabilityErrorRedisUnavailable,

    //
    // Redis metrics
    //
    #[strum(serialize = "error.redis.unavailable")]
    ErrorRedisUnavailable,

    //
    // Database metrics
    //
    // Database metric for retrying the connection
    #[strum(serialize = "database.retry")]
    DatabaseRetry,

    // Database metric for dropping user
    #[strum(serialize = "database.drop_user")]
    DatabaseDropUser,

    //
    // Reliability metrics
    //
    // Reliability gc
    #[strum(serialize = "reliability.gc")]
    ReliabilityGc,
}
