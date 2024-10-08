//! Notification protocol
use std::collections::HashMap;

use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::ms_since_epoch;

#[derive(Serialize, Default, Deserialize, Clone, Debug)]
/// A Publishable Notification record. This is a notification that is either
/// received from a third party or is outbound to a UserAgent. If the
/// UserAgent is not currently available, it may be stored as a
/// [crate::db::NotificationRecord]
pub struct Notification {
    #[serde(rename = "channelID")]
    pub channel_id: Uuid,
    pub version: String,
    #[serde(default = "default_ttl", skip_serializing)]
    pub ttl: u64,
    #[serde(skip_serializing)]
    pub topic: Option<String>,
    #[serde(skip_serializing)]
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing)]
    pub sortkey_timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliability_id: Option<String>,
    #[cfg(feature = "reliable_report")]
    pub reliable_state: Option<crate::reliability::PushReliabilityState>,
}

pub const TOPIC_NOTIFICATION_PREFIX: &str = "01";
pub const STANDARD_NOTIFICATION_PREFIX: &str = "02";

impl Notification {
    /// Return an appropriate chidmessageid
    ///
    /// For standard messages:
    ///     {STANDARD_NOTIFICATION_PREFIX}:{sortkey_timestamp}:{chid}
    ///
    /// For topic messages:
    ///     {TOPIC_NOTIFICATION_PREFIX}:{chid}:{topic}
    ///
    /// Old format for non-topic messages that is no longer returned:
    ///     {chid}:{message_id}
    pub fn chidmessageid(&self) -> String {
        let chid = self.channel_id.as_hyphenated();
        if let Some(ref topic) = self.topic {
            format!("{TOPIC_NOTIFICATION_PREFIX}:{chid}:{topic}")
        } else if let Some(sortkey_timestamp) = self.sortkey_timestamp {
            format!(
                "{STANDARD_NOTIFICATION_PREFIX}:{}:{}",
                if sortkey_timestamp == 0 {
                    ms_since_epoch()
                } else {
                    sortkey_timestamp
                },
                chid
            )
        } else {
            warn!("🚨 LEGACY MESSAGE!? {:?} ", self);
            // Legacy messages which we should never get anymore
            format!("{}:{}", chid, self.version)
        }
    }

    /// Convenience function to determine if the notification
    /// has aged out.
    pub fn expired(&self, at_sec: u64) -> bool {
        at_sec >= self.timestamp + self.ttl
    }
}

fn default_ttl() -> u64 {
    0
}
