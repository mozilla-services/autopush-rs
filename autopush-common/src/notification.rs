//! Notification protocol
use std::collections::HashMap;
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::ms_since_epoch;

#[derive(Serialize, Default, Deserialize, Clone, Debug)]
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
}

impl Notification {
    /// Return an appropriate sort_key to use for the chidmessageid
    ///
    /// For new messages:
    ///     02:{sortkey_timestamp}:{chid}
    ///
    /// For topic messages:
    ///     01:{chid}:{topic}
    ///
    /// Old format for non-topic messages that is no longer returned:
    ///     {chid}:{message_id}
    pub fn sort_key(&self) -> String {
        let chid = self.channel_id.hyphenated();
        if let Some(ref topic) = self.topic {
            format!("01:{}:{}", chid, topic)
        } else if let Some(sortkey_timestamp) = self.sortkey_timestamp {
            format!(
                "02:{}:{}",
                if sortkey_timestamp == 0 {
                    ms_since_epoch()
                } else {
                    sortkey_timestamp
                },
                chid
            )
        } else {
            // Legacy messages which we should never get anymore
            format!("{}:{}", chid, self.version)
        }
    }

    pub fn expired(&self, at_sec: u64) -> bool {
        at_sec >= self.timestamp as u64 + self.ttl as u64
    }
}


/// Semi-generic Message Row to Notification.
impl From<&tokio_postgres::Row> for Notification {
    fn from(row: &tokio_postgres::Row) -> Self {
        Self {
            channel_id: row.try_get::<&str, &str>("channel_id").map(|v| Uuid::from_str(v).unwrap()).unwrap(),
            version: row.try_get::<&str, String>("version").unwrap(),
            ttl: row.try_get::<&str, i64>("ttl").map(|v| v as u64).unwrap(),
            topic: row.try_get::<&str, String>("topic").map(|v| Some(v)).unwrap_or_default(),
            timestamp: row.try_get::<&str, i64>("timestamp").map(|v| v as u64).unwrap(),
            data: row.try_get::<&str, String>("data").map(|v| Some(v)).unwrap(),
            sortkey_timestamp: row.try_get::<&str, i64>("sortkey_timestamp").map(|v| Some(v as u64)).unwrap_or_default(),
            headers: row.try_get::<&str, &str>("headers").map(|v| {
                let hdrs: HashMap<String, String> = serde_json::from_str(v).unwrap();
                Some(hdrs)
            }).unwrap_or_default()
        }
    }
}

fn default_ttl() -> u64 {
    0
}
