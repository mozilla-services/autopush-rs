/// This uses redis as a storage and management
/// system for Autopush Notifications and Routing information.
///
/// Keys for the data are
/// `autopush/user/{uaid}` String to store the user data
/// `autopush/co/{uaid}` u64 to store the last time the user has interacted with the server
/// `autopush/timestamp/{uaid}` u64 to store the last storage timestamp incremented by the server, once messages are delivered
/// `autopush/channels/{uaid}` List to store the list of the channels of the user
/// `autopush/msgs/{uaid}` SortedSet to store the list of the pending message ids for the user
/// `autopush/msgs_exp/{uaid}` SortedSet to store the list of the pending message ids, ordered by expiry date, this is because SortedSet elements can't have independant expiry date
/// `autopush/msg/{uaid}/{chidmessageid}`, with `{chidmessageid} == {chid}:{version}` String to store
/// the content of the messages
///
mod redis_client;

pub use redis_client::RedisClientImpl;

use serde::Deserialize;
use std::time::Duration;

use crate::db::error::DbError;
use crate::util::deserialize_opt_u32_to_duration;

/// The settings for accessing the redis contents.
#[derive(Clone, Debug, Deserialize)]
pub struct RedisDbSettings {
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub timeout: Option<Duration>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub router_ttl: Option<Duration>,
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub notification_ttl: Option<Duration>,
}

// Used by test, but we don't want available for release.
#[allow(clippy::derivable_impls)]
impl Default for RedisDbSettings {
    fn default() -> Self {
        Self {
            timeout: Default::default(),
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
                "Could not parse DdbSettings: {:?}",
                e
            )))?,
        };
        Ok(me)
    }
}

mod tests {

    #[test]
    fn test_settings_parse() -> Result<(), crate::db::error::DbError> {
        let settings = super::RedisDbSettings::try_from("{\"timeout\": 123}")?;
        assert_eq!(settings.timeout, Some(std::time::Duration::from_secs(123)));
        Ok(())
    }
}
