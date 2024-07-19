/// Contains the general Database access bits
///
/// Database access is abstracted into a DbClient impl
/// which contains the required trait functions the
/// application will need to perform in the database.
/// Each of the abstractions contains a DbClientImpl
/// that is responsible for carrying out the requested
/// functions. Each of the data stores are VERY
/// different, although the requested functions
/// are fairly simple.
use std::cmp::min;
use std::collections::{HashMap, HashSet};
use std::result::Result as StdResult;

use derive_builder::Builder;
use lazy_static::lazy_static;
use regex::RegexSet;
use serde::Serializer;
use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "bigtable")]
pub mod bigtable;
pub mod client;
pub mod error;
pub mod models;
pub mod reporter;
pub mod routing;

// used by integration testing
pub mod mock;

pub use reporter::spawn_pool_periodic_reporter;

use crate::errors::{ApcErrorKind, Result};
use crate::notification::{Notification, STANDARD_NOTIFICATION_PREFIX, TOPIC_NOTIFICATION_PREFIX};
use crate::util::timing::{ms_since_epoch, sec_since_epoch};
use models::{NotificationHeaders, RangeKey};

const MAX_EXPIRY: u64 = 2_592_000;
pub const USER_RECORD_VERSION: u64 = 1;
/// The maximum TTL for channels, 30 days
pub const MAX_CHANNEL_TTL: u64 = 30 * 24 * 60 * 60;
/// The maximum TTL for router records, 30 days
pub const MAX_ROUTER_TTL: u64 = MAX_CHANNEL_TTL;

#[derive(Eq, Debug, PartialEq)]
pub enum StorageType {
    INVALID,
    #[cfg(feature = "bigtable")]
    BigTable,
}

impl From<&str> for StorageType {
    fn from(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            #[cfg(feature = "bigtable")]
            "bigtable" => Self::BigTable,
            _ => Self::INVALID,
        }
    }
}

/// The type of storage to use.
#[allow(clippy::vec_init_then_push)] // Because we are only pushing on feature flags.
impl StorageType {
    fn available<'a>() -> Vec<&'a str> {
        #[allow(unused_mut)]
        let mut result: Vec<&str> = Vec::new();
        #[cfg(feature = "bigtable")]
        result.push("Bigtable");
        result
    }

    pub fn from_dsn(dsn: &Option<String>) -> Self {
        debug!("Supported data types: {:?}", StorageType::available());
        debug!("Checking DSN: {:?}", &dsn);
        if dsn.is_none() {
            let default = Self::available()[0];
            info!("No DSN specified, failing over to old default dsn: {default}");
            return Self::from(default);
        }
        let dsn = dsn.clone().unwrap_or_default();
        #[cfg(feature = "bigtable")]
        if dsn.starts_with("grpc") {
            trace!("Found grpc");
            // Credentials can be stored in either a path provided in an environment
            // variable, or $HOME/.config/gcloud/applicaion_default_credentals.json
            //
            // NOTE: if no credentials are found, application will panic
            //
            if let Ok(cred) = std::env::var("GOOGLE_APPLICATION_CREDENTIALS") {
                trace!("Env: {:?}", cred);
            }
            return Self::BigTable;
        }
        Self::INVALID
    }
}

/// The universal settings for the database
/// abstractor.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct DbSettings {
    /// Database connector string
    pub dsn: Option<String>,
    /// A JSON formatted dictionary containing Database settings that
    /// are specific to the type of Data storage specified in the `dsn`
    /// See the respective settings structure for
    /// [crate::db::bigtable::BigTableDbSettings]
    pub db_settings: String,
}
//TODO: add `From<autopush::settings::Settings> for DbSettings`?
//TODO: add `From<autoendpoint::settings::Settings> for DbSettings`?

/// Custom Uuid serializer
///
/// Serializes a Uuid as a simple string instead of hyphenated
pub fn uuid_serializer<S>(x: &Uuid, s: S) -> StdResult<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_str(&x.simple().to_string())
}

#[derive(Clone, Default, Debug)]
pub struct CheckStorageResponse {
    /// The messages include a "topic"
    /// "topics" are messages that replace prior messages of that topic.
    /// (e.g. you can only have one message for a topic of "foo")
    pub include_topic: bool,
    /// The list of pending messages.
    pub messages: Vec<Notification>,
    /// All the messages up to this timestamp
    pub timestamp: Option<u64>,
}

/// A user data record.
#[derive(Deserialize, PartialEq, Debug, Clone, Serialize, Builder)]
#[builder(default, setter(strip_option))]
pub struct User {
    /// The UAID. This is generally a UUID4. It needs to be globally
    /// unique.
    #[serde(serialize_with = "uuid_serializer")]
    pub uaid: Uuid,
    /// Time in milliseconds that the user last connected at
    pub connected_at: u64,
    /// Router type of the user
    pub router_type: String,
    /// Router-specific data
    pub router_data: Option<HashMap<String, serde_json::Value>>,
    /// Last node/port the client was or may be connected to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Record version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_version: Option<u64>,
    /// the timestamp of the last notification sent to the user
    /// This field is exclusive to the Bigtable data scheme
    //TODO: rename this to `last_notification_timestamp`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_timestamp: Option<u64>,
    /// UUID4 version number for optimistic locking of updates on Bigtable
    #[serde(skip_serializing)]
    pub version: Option<Uuid>,
    /// Set of user's channel ids. These are stored in router (user) record's
    /// row in Bigtable. They are read along with the rest of the user record
    /// so that them, along with every other field in the router record, will
    /// automatically have their TTL (cell timestamp) reset during
    /// [DbClient::update_user].
    ///
    /// This is solely used for the sake of that update thus private.
    /// [DbClient::get_channels] is preferred for reading the latest version of
    /// the channel ids (partly due to historical purposes but also is a more
    /// flexible API that might benefit different, non Bigtable [DbClient]
    /// backends that don't necessarily store the channel ids in the router
    /// record).
    _channels: HashSet<Uuid>,
}

impl Default for User {
    fn default() -> Self {
        let uaid = Uuid::new_v4();
        //trace!(">>> Setting default uaid: {:?}", &uaid);
        Self {
            uaid,
            connected_at: ms_since_epoch(),
            router_type: "webpush".to_string(),
            router_data: None,
            node_id: None,
            record_version: Some(USER_RECORD_VERSION),
            current_timestamp: None,
            version: Some(Uuid::new_v4()),
            _channels: HashSet::new(),
        }
    }
}

impl User {
    /// Return a new [UserBuilder] (generated from [derive_builder::Builder])
    pub fn builder() -> UserBuilder {
        UserBuilder::default()
    }
}

/// A stored Notification record. This is a notification that is to be stored
/// until the User Agent reconnects. These are then converted to publishable
/// [crate::db::Notification] records.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct NotificationRecord {
    /// The UserAgent Identifier (UAID)
    #[serde(serialize_with = "uuid_serializer")]
    uaid: Uuid,
    // Format:
    //    Topic Messages:
    //        {TOPIC_NOTIFICATION_PREFIX}:{channel id}:{topic}
    //    New Messages:
    //        {STANDARD_NOTIFICATION_PREFIX}:{timestamp int in microseconds}:{channel id}
    chidmessageid: String,
    /// Magic entry stored in the first Message record that indicates the highest
    /// non-topic timestamp we've read into
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_timestamp: Option<u64>,
    /// Magic entry stored in the first Message record that indicates the valid
    /// channel id's
    #[serde(skip_serializing)]
    pub chids: Option<HashSet<String>>,
    /// Time in seconds from epoch
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<u64>,
    /// Expiration timestamp
    expiry: u64,
    /// TTL value provided by application server for the message
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<u64>,
    /// The message data
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    /// Selected, associated message headers. These can contain additional
    /// decryption information for the UserAgent.
    #[serde(skip_serializing_if = "Option::is_none")]
    headers: Option<NotificationHeaders>,
    /// This is the acknowledgement-id used for clients to ack that they have received the
    /// message. Autoendpoint refers to this as a message_id. Endpoints generate this
    /// value before sending it to storage or a connection node.
    #[serde(skip_serializing_if = "Option::is_none")]
    updateid: Option<String>,
}

impl NotificationRecord {
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

    /// Convert the stored notifications into publishable notifications
    pub fn into_notif(self) -> Result<Notification> {
        let key = Self::parse_chidmessageid(&self.chidmessageid)?;
        let version = key
            .legacy_version
            .or(self.updateid)
            .ok_or(ApcErrorKind::GeneralError(
                "No valid updateid/version found".into(),
            ))?;

        Ok(Notification {
            channel_id: key.channel_id,
            version,
            ttl: self.ttl.unwrap_or(0),
            timestamp: self
                .timestamp
                .ok_or("No timestamp found")
                .map_err(|e| ApcErrorKind::GeneralError(e.to_string()))?,
            topic: key.topic,
            data: self.data,
            headers: self.headers.map(|m| m.into()),
            sortkey_timestamp: key.sortkey_timestamp,
        })
    }

    /// Convert from a publishable Notification to a stored notification
    pub fn from_notif(uaid: &Uuid, val: Notification) -> Self {
        Self {
            uaid: *uaid,
            chidmessageid: val.chidmessageid(),
            timestamp: Some(val.timestamp),
            expiry: sec_since_epoch() + min(val.ttl, MAX_EXPIRY),
            ttl: Some(val.ttl),
            data: val.data,
            headers: val.headers.map(|h| h.into()),
            updateid: Some(val.version),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{User, USER_RECORD_VERSION};

    #[test]
    fn user_defaults() {
        let user = User::builder().current_timestamp(22).build().unwrap();
        assert_eq!(user.current_timestamp, Some(22));
        assert_eq!(user.router_type, "webpush".to_owned());
        assert_eq!(user.record_version, Some(USER_RECORD_VERSION));
    }
}
