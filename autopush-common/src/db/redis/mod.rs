/// This uses redis as a storage and management
/// system for Autopush Notifications and Routing information.
///
/// Keys for the data are
/// `autopush/user/{uaid}` String to store the user data
/// `autopush/co/{uaid}` u64 to store the last time the user has interacted with the server
/// `autopush/timestamp/{uaid}` u64 to store the last storage timestamp incremented by the server, once messages are delivered
/// `autopush/channels/{uaid}` List to store the list of the channels of the user
/// `autopush/msgs/{uaid}` SortedSet to store the list of the pending message ids for the user
/// `autopush/msgs_exp/{uaid}` SortedSet to store the list of the pending message ids, ordered by expiry date, this is because SortedSet elements can't have independent expiry date
/// `autopush/msg/{uaid}/{chidmessageid}`, with `{chidmessageid} == {chid}:{version}` String to store
/// the content of the messages
///
mod redis_client;

pub use redis_client::RedisClientImpl;

use std::collections::HashMap;
use std::time::Duration;

use crate::db::error::DbError;
use crate::notification::{default_ttl, Notification};
use crate::util::deserialize_opt_u32_to_duration;

use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

/// The settings for accessing the redis contents.
#[derive(Clone, Debug, Deserialize)]
pub struct RedisDbSettings {
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub create_timeout: Option<Duration>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub router_ttl: Option<Duration>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub notification_ttl: Option<Duration>,
}

#[allow(clippy::derivable_impls)]
impl Default for RedisDbSettings {
    fn default() -> Self {
        Self {
            create_timeout: Default::default(),
            router_ttl: Some(Duration::from_secs(crate::MAX_ROUTER_TTL_SECS)),
            notification_ttl: Some(Duration::from_secs(crate::MAX_NOTIFICATION_TTL_SECS)),
        }
    }
}

impl TryFrom<&str> for RedisDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        let me: Self = match serde_json::from_str(setting_string) {
            Ok(me) => me,
            Err(e) if e.is_eof() => Self::default(),
            Err(e) => Err(DbError::General(format!(
                "Could not parse RedisDbSettings: {:?}",
                e
            )))?,
        };
        Ok(me)
    }
}

#[derive(Serialize, Default, Deserialize, Clone, Debug)]
/// A Publishable Notification record. This is a notification that is either
/// received from a third party or is outbound to a UserAgent.
///
pub struct StorableNotification {
    // Required values
    #[serde(rename = "channelID")]
    pub channel_id: Uuid,
    pub version: String,
    pub timestamp: u64,
    // Possibly stored values, provided with a default.
    #[serde(default = "default_ttl", skip_serializing)]
    pub ttl: u64,
    // Optional values, which imply a "None" default.
    #[serde(skip_serializing)]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing)]
    pub sortkey_timestamp: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    #[cfg(feature = "reliable_report")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliability_id: Option<String>,
    #[cfg(feature = "reliable_report")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reliable_state: Option<crate::reliability::ReliabilityState>,
}

impl From<Notification> for StorableNotification {
    fn from(notification: Notification) -> Self {
        Self {
            channel_id: notification.channel_id,
            version: notification.version,
            timestamp: notification.timestamp,
            ttl: notification.ttl,
            topic: notification.topic,
            data: notification.data,
            sortkey_timestamp: notification.sortkey_timestamp,
            headers: notification.headers,
            #[cfg(feature = "reliable_report")]
            reliability_id: notification.reliability_id,
            #[cfg(feature = "reliable_report")]
            reliable_state: notification.reliable_state,
        }
    }
}

impl From<StorableNotification> for Notification {
    fn from(storable: StorableNotification) -> Self {
        Self {
            channel_id: storable.channel_id,
            version: storable.version,
            timestamp: storable.timestamp,
            ttl: storable.ttl,
            topic: storable.topic,
            data: storable.data,
            sortkey_timestamp: storable.sortkey_timestamp,
            headers: storable.headers,
            #[cfg(feature = "reliable_report")]
            reliability_id: storable.reliability_id,
            #[cfg(feature = "reliable_report")]
            reliable_state: storable.reliable_state,
        }
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_settings_parse() -> Result<(), crate::db::error::DbError> {
        let settings = super::RedisDbSettings::try_from("{\"timeout\": 123}")?;
        assert_eq!(
            settings.create_timeout,
            Some(std::time::Duration::from_secs(123))
        );
        Ok(())
    }
}
