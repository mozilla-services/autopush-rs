use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::result::Result as StdResult;
use uuid::Uuid;

use regex::RegexSet;
use serde::Serializer;

use db::util::generate_last_connect;
use errors::*;
use protocol::Notification;
use util::timing::{ms_since_epoch, sec_since_epoch};

use super::{MAX_EXPIRY, USER_RECORD_VERSION};

/// Custom Uuid serializer
///
/// Serializes a Uuid as a simple string instead of hyphenated
fn uuid_serializer<S>(x: &Uuid, s: S) -> StdResult<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_str(&x.simple().to_string())
}

/// Direct representation of a DynamoDB Notification as we store it in the database
/// Most attributes are optional
#[derive(Default, Deserialize, PartialEq, Debug, Clone, Serialize)]
struct NotificationHeaders {
    #[serde(skip_serializing_if = "Option::is_none")]
    crypto_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encryption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encryption_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    encoding: Option<String>,
}

fn insert_to_map(map: &mut HashMap<String, String>, name: &str, val: Option<String>) {
    if let Some(val) = val {
        map.insert(name.to_string(), val);
    }
}

impl From<NotificationHeaders> for HashMap<String, String> {
    fn from(val: NotificationHeaders) -> Self {
        let mut map = Self::new();
        insert_to_map(&mut map, "crypto_key", val.crypto_key);
        insert_to_map(&mut map, "encryption", val.encryption);
        insert_to_map(&mut map, "encryption_key", val.encryption_key);
        insert_to_map(&mut map, "encoding", val.encoding);
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

#[derive(Deserialize, PartialEq, Debug, Clone, Serialize)]
pub struct DynamoDbUser {
    // DynamoDB <Hash key>
    #[serde(serialize_with = "uuid_serializer")]
    pub uaid: Uuid,
    // Time in milliseconds that the user last connected at
    pub connected_at: u64,
    // Router type of the user
    pub router_type: String,
    // Keyed time in a month the user last connected at with limited key range for indexing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_connect: Option<u64>,
    // Last node/port the client was or may be connected to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    // Record version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_version: Option<u8>,
    // Current month table in the database the user is on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_month: Option<String>,
}

impl Default for DynamoDbUser {
    fn default() -> Self {
        Self {
            uaid: Uuid::new_v4(),
            connected_at: ms_since_epoch(),
            router_type: "webpush".to_string(),
            last_connect: Some(generate_last_connect()),
            node_id: None,
            record_version: Some(USER_RECORD_VERSION),
            current_month: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DynamoDbNotification {
    // DynamoDB <Hash key>
    #[serde(serialize_with = "uuid_serializer")]
    uaid: Uuid,
    // DynamoDB <Range key>
    // Format:
    //    Topic Messages:
    //        01:{channel id}:{topic}
    //    New Messages:
    //        02:{timestamp int in microseconds}:{channel id}
    chidmessageid: String,
    // Magic entry stored in the first Message record that indicates the highest
    // non-topic timestamp we've read into
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_timestamp: Option<u64>,
    // Magic entry stored in the first Message record that indicates the valid
    // channel id's
    #[serde(skip_serializing)]
    pub chids: Option<HashSet<String>>,
    // Time in seconds from epoch
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<u32>,
    // DynamoDB expiration timestamp per
    //    https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/TTL.html
    expiry: u32,
    // TTL value provided by application server for the message
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<NotificationHeaders>,
    // This is the acknowledgement-id used for clients to ack that they have received the
    // message. Some Python code refers to this as a message_id. Endpoints generate this
    // value before sending it to storage or a connection node.
    #[serde(skip_serializing_if = "Option::is_none")]
    updateid: Option<String>,
}

impl DynamoDbNotification {
    fn parse_sort_key(key: &str) -> Result<RangeKey> {
        lazy_static! {
            static ref RE: RegexSet =
                RegexSet::new(&[r"^01:\S+:\S+$", r"^02:\d+:\S+$", r"^\S{3,}:\S+$",]).unwrap();
        }
        if !RE.is_match(key) {
            return Err("Invalid chidmessageid".into());
        }

        let v: Vec<&str> = key.split(':').collect();
        match v[0] {
            "01" => {
                if v.len() != 3 {
                    return Err("Invalid topic key".into());
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
            "02" => {
                if v.len() != 3 {
                    return Err("Invalid topic key".into());
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
            _ => {
                if v.len() != 2 {
                    return Err("Invalid topic key".into());
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

    // TODO: Implement as TryFrom whenever that lands
    pub fn into_notif(self) -> Result<Notification> {
        let key = Self::parse_sort_key(&self.chidmessageid)?;
        let version = key
            .legacy_version
            .or(self.updateid)
            .ok_or("No valid updateid/version found")?;

        Ok(Notification {
            channel_id: key.channel_id,
            version,
            ttl: self.ttl.ok_or("No TTL found")?,
            timestamp: self.timestamp.ok_or("No timestamp found")?,
            topic: key.topic,
            data: self.data,
            headers: self.headers.map(|m| m.into()),
            sortkey_timestamp: key.sortkey_timestamp,
        })
    }

    pub fn from_notif(uaid: &Uuid, val: Notification) -> Self {
        Self {
            uaid: *uaid,
            chidmessageid: val.sort_key(),
            timestamp: Some(val.timestamp),
            expiry: sec_since_epoch() as u32 + min(val.ttl, MAX_EXPIRY as u32),
            ttl: Some(val.ttl),
            data: val.data,
            headers: val.headers.map(|h| h.into()),
            updateid: Some(val.version),
            ..Default::default()
        }
    }
}

struct RangeKey {
    channel_id: Uuid,
    topic: Option<String>,
    pub sortkey_timestamp: Option<u64>,
    legacy_version: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::DynamoDbNotification;
    use util::us_since_epoch;
    use uuid::Uuid;

    #[test]
    fn test_parse_sort_key_ver1() {
        let chid = Uuid::new_v4();
        let chidmessageid = format!("01:{}:mytopic", chid.hyphenated().to_string());
        let key = DynamoDbNotification::parse_sort_key(&chidmessageid).unwrap();
        assert_eq!(key.topic, Some("mytopic".to_string()));
        assert_eq!(key.channel_id, chid);
        assert_eq!(key.sortkey_timestamp, None);
    }

    #[test]
    fn test_parse_sort_key_ver2() {
        let chid = Uuid::new_v4();
        let sortkey_timestamp = us_since_epoch();
        let chidmessageid = format!("02:{}:{}", sortkey_timestamp, chid.hyphenated().to_string());
        let key = DynamoDbNotification::parse_sort_key(&chidmessageid).unwrap();
        assert_eq!(key.topic, None);
        assert_eq!(key.channel_id, chid);
        assert_eq!(key.sortkey_timestamp, Some(sortkey_timestamp));
    }

    #[test]
    fn test_parse_sort_key_bad_values() {
        for val in vec!["02j3i2o", "03:ffas:wef", "01::mytopic", "02:oops:ohnoes"] {
            let key = DynamoDbNotification::parse_sort_key(val);
            assert!(key.is_err());
        }
    }
}
