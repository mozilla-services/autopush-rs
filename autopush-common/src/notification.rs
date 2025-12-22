//! Notification protocol
use std::collections::HashMap;
#[cfg(feature="postgres")]
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::ms_since_epoch;

#[derive(Serialize, Default, Deserialize, Clone, Debug)]
/// A Publishable Notification record. This is a notification that is either
/// received from a third party or is outbound to a UserAgent.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliable_state: Option<crate::reliability::ReliabilityState>,
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

    pub fn expiry(&self) -> u64 {
        self.timestamp + self.ttl
    }

    /// Convenience function to determine if the notification
    /// has aged out.
    pub fn expired(&self, at_sec: u64) -> bool {
        at_sec >= self.expiry()
    }

    #[cfg(feature = "reliable_report")]
    pub async fn record_reliability(
        &mut self,
        reliability: &crate::reliability::PushReliability,
        state: crate::reliability::ReliabilityState,
    ) {
        self.reliable_state = reliability
            .record(
                &self.reliability_id,
                state,
                &self.reliable_state,
                Some(self.expiry()),
            )
            .await
            .inspect_err(|e| {
                warn!("🔍⚠️ Unable to record reliability state log: {:?}", e);
            })
            .unwrap_or(Some(state));
    }

    #[cfg(feature = "reliable_report")]
    pub fn clone_without_reliability_state(&self) -> Self {
        let mut cloned = self.clone();
        cloned.reliable_state = None;
        cloned
    }
}

fn default_ttl() -> u64 {
    0
}

#[cfg(feature="postgres")]
/// Semi-generic Message Row to Notification.
impl From<&tokio_postgres::Row> for Notification {
    fn from(row: &tokio_postgres::Row) -> Self {
        use crate::reliability::ReliabilityState;
        Self {
            channel_id: row
                .try_get::<&str, &str>("channel_id")
                .map(|v| Uuid::from_str(v).unwrap())
                .unwrap(),
            version: row.try_get::<&str, String>("version").unwrap(),
            ttl: row.try_get::<&str, i64>("ttl").map(|v| v as u64).unwrap(),
            topic: row
                .try_get::<&str, String>("topic")
                .map(Some)
                .unwrap_or_default(),
            timestamp: row
                .try_get::<&str, i64>("timestamp")
                .map(|v| v as u64)
                .unwrap(),
            data: row
                .try_get::<&str, String>("data")
                .map(Some)
                .unwrap(),
            sortkey_timestamp: row
                .try_get::<&str, i64>("sortkey_timestamp")
                .map(|v| Some(v as u64))
                .unwrap_or_default(),
            headers: row
                .try_get::<&str, &str>("headers")
                .map(|v| {
                    let hdrs: HashMap<String, String> = serde_json::from_str(v).unwrap();
                    Some(hdrs)
                })
                .unwrap_or_default(),
            reliability_id: row
                .try_get::<&str, String>("reliability_id").ok(),
            reliable_state: row.try_get::<&str, &str>("reliable_state")
                .map(|v| ReliabilityState::from_str(v).unwrap()).ok()
        }
    }
}
