use regex::RegexSet;
use std::collections::HashMap;
use uuid::Uuid;

use crate::errors::{ApcErrorKind, Result};
use crate::notification::{Notification, STANDARD_NOTIFICATION_PREFIX, TOPIC_NOTIFICATION_PREFIX};
use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use crate::util::InsertOpt;

/// Direct representation of an incoming subscription notification header set
/// as we store it in the database.
/// It is possible to have a "data free" notification, which does not have a
/// message component, and thus, no headers.
#[derive(Default, Deserialize, PartialEq, Debug, Clone, Serialize)]
pub(crate) struct NotificationHeaders {
    #[serde(skip_serializing_if = "Option::is_none")]
    crypto_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encryption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encryption_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<String>,
}

#[allow(clippy::implicit_hasher)]
impl From<NotificationHeaders> for HashMap<String, String> {
    fn from(val: NotificationHeaders) -> Self {
        let mut map = Self::new();
        map.insert_opt("crypto_key", val.crypto_key);
        map.insert_opt("encryption", val.encryption);
        map.insert_opt("encryption_key", val.encryption_key);
        map.insert_opt("encoding", val.encoding);
        map
    }
}

impl From<HashMap<String, String>> for NotificationHeaders {
    fn from(val: HashMap<String, String>) -> Self {
        Self {
            crypto_key: val.get("crypto_key").map(|v| v.to_string()),
            encryption: val.get("encryption").map(|v| v.to_string()),
            encryption_key: val.get("encryption_key").map(|v| v.to_string()),
            encoding: val.get("encoding").map(|v| v.to_string()),
        }
    }
}

/// Contains some meta info regarding the message we're handling.
#[derive(Debug)]
pub(crate) struct RangeKey {
    /// The channel_identifier
    pub(crate) channel_id: Uuid,
    /// The optional topic identifier
    pub(crate) topic: Option<String>,
    /// The encoded sortkey and timestamp
    pub(crate) sortkey_timestamp: Option<u64>,
    /// Which version of this message are we handling
    pub(crate) legacy_version: Option<String>,
}

impl RangeKey {
    /// read the custom sort_key and convert it into something the database can use.
    pub(crate) fn parse_chidmessageid(key: &str) -> Result<RangeKey> {
        lazy_static! {
            static ref RE: RegexSet = RegexSet::new([
                format!("^{}:\\S+:\\S+$", TOPIC_NOTIFICATION_PREFIX).as_str(),
                format!("^{}:\\d+:\\S+$", STANDARD_NOTIFICATION_PREFIX).as_str(),
                "^\\S{3,}:\\S+$"
            ])
            .unwrap();
        }
        if !RE.is_match(key) {
            return Err(ApcErrorKind::GeneralError("Invalid chidmessageid".into()).into());
        }

        let v: Vec<&str> = key.split(':').collect();
        match v[0] {
            // This is a topic message (There Can Only Be One. <guitar riff>)
            "01" => {
                if v.len() != 3 {
                    return Err(ApcErrorKind::GeneralError("Invalid topic key".into()).into());
                }
                let (channel_id, topic) = (v[1], v[2]);
                let channel_id = Uuid::parse_str(channel_id)?;
                Ok(RangeKey {
                    channel_id,
                    topic: Some(topic.to_string()),
                    sortkey_timestamp: None,
                    legacy_version: None,
                })
            }
            // A "normal" pending message.
            "02" => {
                if v.len() != 3 {
                    return Err(ApcErrorKind::GeneralError("Invalid topic key".into()).into());
                }
                let (sortkey, channel_id) = (v[1], v[2]);
                let channel_id = Uuid::parse_str(channel_id)?;
                Ok(RangeKey {
                    channel_id,
                    topic: None,
                    sortkey_timestamp: Some(sortkey.parse()?),
                    legacy_version: None,
                })
            }
            // Ok, that's odd, but try to make some sense of it.
            // (This is a bit of legacy code that we should be
            // able to drop.)
            _ => {
                if v.len() != 2 {
                    return Err(ApcErrorKind::GeneralError("Invalid topic key".into()).into());
                }
                let (channel_id, legacy_version) = (v[0], v[1]);
                let channel_id = Uuid::parse_str(channel_id)?;
                Ok(RangeKey {
                    channel_id,
                    topic: None,
                    sortkey_timestamp: None,
                    legacy_version: Some(legacy_version.to_string()),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::NotificationRecord;
    use crate::util::us_since_epoch;
    use uuid::Uuid;

    #[test]
    fn test_parse_sort_key_ver1() {
        let chid = Uuid::new_v4();
        let chidmessageid = format!("01:{}:mytopic", chid.hyphenated());
        let key = NotificationRecord::parse_chidmessageid(&chidmessageid).unwrap();
        assert_eq!(key.topic, Some("mytopic".to_string()));
        assert_eq!(key.channel_id, chid);
        assert_eq!(key.sortkey_timestamp, None);
    }

    #[test]
    fn test_parse_sort_key_ver2() {
        let chid = Uuid::new_v4();
        let sortkey_timestamp = us_since_epoch();
        let chidmessageid = format!("02:{}:{}", sortkey_timestamp, chid.hyphenated());
        let key = NotificationRecord::parse_chidmessageid(&chidmessageid).unwrap();
        assert_eq!(key.topic, None);
        assert_eq!(key.channel_id, chid);
        assert_eq!(key.sortkey_timestamp, Some(sortkey_timestamp));
    }

    #[test]
    fn test_parse_sort_key_bad_values() {
        for val in &["02j3i2o", "03:ffas:wef", "01::mytopic", "02:oops:ohnoes"] {
            let key = NotificationRecord::parse_chidmessageid(val);
            assert!(key.is_err());
        }
    }
}
