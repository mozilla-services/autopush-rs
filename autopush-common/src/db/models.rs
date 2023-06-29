use std::collections::HashMap;

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

#[cfg(test)]
mod tests {
    use crate::db::NotificationRecord;
    use crate::util::us_since_epoch;
    use uuid::Uuid;

    #[test]
    fn test_parse_sort_key_ver1() {
        let chid = Uuid::new_v4();
        let chidmessageid = format!("01:{}:mytopic", chid.hyphenated());
        let key = NotificationRecord::parse_chid_msgid(&chidmessageid).unwrap();
        assert_eq!(key.topic, Some("mytopic".to_string()));
        assert_eq!(key.channel_id, chid);
        assert_eq!(key.sortkey_timestamp, None);
    }

    #[test]
    fn test_parse_sort_key_ver2() {
        let chid = Uuid::new_v4();
        let sortkey_timestamp = us_since_epoch();
        let chidmessageid = format!("02:{}:{}", sortkey_timestamp, chid.hyphenated());
        let key = NotificationRecord::parse_chid_msgid(&chidmessageid).unwrap();
        assert_eq!(key.topic, None);
        assert_eq!(key.channel_id, chid);
        assert_eq!(key.sortkey_timestamp, Some(sortkey_timestamp));
    }

    #[test]
    fn test_parse_sort_key_bad_values() {
        for val in &["02j3i2o", "03:ffas:wef", "01::mytopic", "02:oops:ohnoes"] {
            let key = NotificationRecord::parse_chid_msgid(val);
            assert!(key.is_err());
        }
    }
}
