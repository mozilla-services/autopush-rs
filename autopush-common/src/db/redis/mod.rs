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
use crate::notification::{Notification, default_ttl};
use crate::util::deserialize_opt_u32_to_duration;

use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

/// The settings for accessing the redis contents.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RedisDbSettings {
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub create_timeout: Option<Duration>,
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    // Minimum value is 1 (second), defaults to MAX_ROUTER_TTL_SECS
    pub router_ttl: Option<Duration>,
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    // Minimum value is 1 (second), defaults to MAX_NOTIFICATION_TTL_SECS
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
        if let Some(router_ttl) = me.router_ttl
            && router_ttl.as_secs() == 0
        {
            return Err(DbError::General(
                "router_ttl must be greater than 0".to_string(),
            ));
        }
        if let Some(notification_ttl) = me.notification_ttl
            && notification_ttl.as_secs() == 0
        {
            return Err(DbError::General(
                "notification_ttl must be greater than 0".to_string(),
            ));
        }
        // Supply defaults for explicitly null values (deserializer handles missing keys)
        // Otherwise it defaults to 0 duration, which is not a valid TTL
        let me = Self {
            router_ttl: me
                .router_ttl
                .or(Some(Duration::from_secs(crate::MAX_ROUTER_TTL_SECS))),
            notification_ttl: me
                .notification_ttl
                .or(Some(Duration::from_secs(crate::MAX_NOTIFICATION_TTL_SECS))),
            ..me
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
    // Note: Unlike client-facing `Notification`, these fields
    // should round-trip faithfully through Redis and not
    // `skip_serializing` unless truly None.
    #[serde(default = "default_ttl")]
    pub ttl: u64,
    // Optional values, which imply a "None" default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

    use std::time::Duration;

    /// A stored notification must round-trip every field through JSON: dropping
    /// `ttl`, `topic`, or `sortkey_timestamp` makes fetched records look like
    /// expired legacy messages, breaking deletion and delivery (autopush-rs#1189).
    #[test]
    fn test_storable_notification_roundtrip() {
        use super::StorableNotification;
        use crate::notification::Notification;
        use uuid::Uuid;

        // A regular (non-topic) timestamp message.
        let notif = Notification {
            channel_id: Uuid::parse_str("DECAFBAD-0000-0000-0000-0123456789AB").unwrap(),
            version: "gAAAAAdeadbeef".to_owned(),
            ttl: 300,
            timestamp: 1_700_000_000,
            data: Some("encrypted".to_owned()),
            sortkey_timestamp: Some(1_700_000_000_123),
            ..Default::default()
        };
        let expected_id = notif.chidmessageid();

        let stored: StorableNotification = notif.into();
        let json = serde_json::to_string(&stored).unwrap();
        let back: Notification = serde_json::from_str::<StorableNotification>(&json)
            .unwrap()
            .into();

        assert_eq!(back.ttl, 300);
        assert_eq!(back.sortkey_timestamp, Some(1_700_000_000_123));
        assert_eq!(back.topic, None);
        // The id used for storage/deletion must survive the round-trip, and must
        // not degrade into the legacy `{chid}:{version}` form.
        assert_eq!(back.chidmessageid(), expected_id);
        assert!(back.chidmessageid().starts_with("02:"));

        // A topic message.
        let topic_notif = Notification {
            channel_id: Uuid::parse_str("DECAFBAD-1111-0000-0000-0123456789AB").unwrap(),
            version: "gAAAAAtopic".to_owned(),
            ttl: 60,
            timestamp: 1_700_000_000,
            topic: Some("mytopic".to_owned()),
            data: Some("encrypted".to_owned()),
            ..Default::default()
        };
        let expected_topic_id = topic_notif.chidmessageid();
        let stored: StorableNotification = topic_notif.into();
        let json = serde_json::to_string(&stored).unwrap();
        let back: Notification = serde_json::from_str::<StorableNotification>(&json)
            .unwrap()
            .into();
        assert_eq!(back.topic, Some("mytopic".to_owned()));
        assert_eq!(back.chidmessageid(), expected_topic_id);
        assert!(back.chidmessageid().starts_with("01:"));
    }

    #[test]
    fn test_settings_parse() -> Result<(), crate::db::error::DbError> {
        let settings = super::RedisDbSettings::try_from("{\"create_timeout\": 123}")?;
        assert_eq!(
            settings.create_timeout,
            Some(std::time::Duration::from_secs(123))
        );
        let settings = super::RedisDbSettings::try_from("{}")?;
        assert_ne!(settings.router_ttl, Some(Duration::from_secs(0)));
        assert_ne!(settings.notification_ttl, Some(Duration::from_secs(0)));
        let settings = super::RedisDbSettings::try_from("{\"router_ttl\":0}");
        assert!(settings.is_err());
        let settings =
            super::RedisDbSettings::try_from("{\"notification_ttl\": null, \"router_ttl\": null}")?;
        assert_eq!(
            settings.notification_ttl,
            Some(std::time::Duration::from_secs(
                crate::MAX_NOTIFICATION_TTL_SECS
            ))
        );
        assert_eq!(
            settings.router_ttl,
            Some(std::time::Duration::from_secs(crate::MAX_ROUTER_TTL_SECS))
        );
        Ok(())
    }
}
