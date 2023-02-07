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

use async_trait::async_trait;
use lazy_static::lazy_static;
use regex::RegexSet;
use serde::Serializer;
use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::util::generate_last_connect;

#[macro_use]
mod macros;
mod commands;

pub mod client;
pub mod dynamodb;
pub mod error;
pub mod models;
//pub mod bigtable;
//pub mod postgres;
mod util;

use crate::errors::{ApcErrorKind, Result};
use crate::notification::Notification;
use crate::util::timing::{ms_since_epoch, sec_since_epoch};
use models::{NotificationHeaders, RangeKey};

const MAX_EXPIRY: u64 = 2_592_000;
const USER_RECORD_VERSION: u8 = 1;
/// The maximum TTL for channels, 30 days
pub const MAX_CHANNEL_TTL: u64 = 30 * 24 * 60 * 60;

#[derive(Eq, PartialEq)]
pub enum StorageType {
    DYNAMODB,
}

impl StorageType {
    /// currently, there is only one.
    pub fn from_dsn(_dsn: &Option<String>) -> Self {
        /*
        if let Some(dsn) = _dsn {
            if dsn.starts_with("http") {
                Self::DYNAMODB
            }
        }
        */
        Self::DYNAMODB
    }
}

/// The universal settings for the database
/// abstractor.
#[derive(Clone, Debug, Default)]
pub struct DbSettings {
    /// Database connector string
    pub dsn: Option<String>,
    /// A JSON formatted dictionary containing Database settings that
    /// are specific to the type of Data storage specified in the `dsn`
    /// See the respective settings structures for
    /// [crate::db::bigtable::BigTableDbSettings], [crate::db::dynamodb::DynamoDbSettings],
    /// [crate::db::postgres::PostgresDbSettings]
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

/// DbCommandClient trait
/// Define a set of traits for handling the data access portions of the
/// various commands for the endpoint
#[async_trait]
pub trait DbCommandClient: Send + Sync {
    //*
    /// `hello` registers a new UAID or records a
    /// returning UAID.
    async fn hello(
        &self,
        // when the UAID connected
        connected_at: u64,
        // either the returning UAID or a request for a new one.
        uaid: Option<&Uuid>,
        // The router that the UAID connected to
        router_url: &str,
        // Hold of actually registering this.
        // (Some UAIDs only connect to rec'v broadcast messages)
        defer_registration: bool,
    ) -> Result<HelloResponse>;

    /// register a new channel for this UAID
    async fn register(
        &self,
        // The requesting UAID
        uaid: &Uuid,
        // The incoming channelID (Note: we must maintain this ID the
        // same way that we recv'd it. (e.g. with dashes or not, case,
        // etc.) )
        channel_id: &Uuid,
        // Legacy field from table rotation
        message_month: &str,
        // TODO??
        endpoint: &str,
        // The user record associated with this UAID
        register_user: Option<&UserRecord>,
    ) -> Result<RegisterResponse>;

    /// Delete this user and all information associated with them
    async fn drop_uaid(&self, uaid: &Uuid) -> Result<()>;

    /// Delete this Channel ID for the user
    async fn unregister(&self, uaid: &Uuid, channel_id: &Uuid, message_month: &str)
        -> Result<bool>;

    /// LEGACY: move this user to the most recent message table
    async fn migrate_user(&self, uaid: &Uuid, message_month: &str) -> Result<()>;

    /// store the message for this user
    async fn store_message(
        &self,
        uaid: &Uuid,
        message_month: String,
        message: Notification,
    ) -> Result<()>;

    /// store multiple messages for this user.
    async fn store_messages(
        &self,
        uaid: &Uuid,
        message_month: &str,
        messages: Vec<Notification>,
    ) -> Result<()>;

    /// Delete a message for this user.
    async fn delete_message(
        &self,
        table_name: &str,
        uaid: &Uuid,
        notif: &Notification,
    ) -> Result<()>;

    /// Fetch any pending messages for this user
    async fn check_storage(
        &self,
        table_name: &str,
        uaid: &Uuid,
        include_topic: bool,
        timestamp: Option<u64>,
    ) -> Result<CheckStorageResponse>;

    /// Get a list of known channels for this user.
    /// (Used by daily mobile client check-in)
    async fn get_user_channels(&self, uaid: &Uuid, message_table: &str) -> Result<HashSet<Uuid>>;

    /// Remove the node information for this user (the
    /// user has disconnected from a node and is considered
    /// inactive or logged out.)
    async fn remove_node_id(&self, uaid: &Uuid, node_id: String, connected_at: u64) -> Result<()>;
    // */
}

/// Basic requirements for notification content to deliver to websocket client
///  - channelID  (the subscription website intended for)
///  - version    (only really utilized for notification acknowledgement in
///                webpush, used to be the sole carrier of data, can now be anything)
///  - data       (encrypted content)
///  - headers    (hash of crypto headers: encoding, encrypption, crypto-key, encryption-key)
#[derive(Default, Clone)]
pub struct HelloResponse {
    /// The UAID the client should use.
    pub uaid: Option<Uuid>,
    /// LEGACY the message month for this user
    pub message_month: String,
    /// Do you need to fetch pending messages.
    pub check_storage: bool,
    /// Give the UA a new ID.
    pub reset_uaid: bool,
    /// LEGACY move the user to a new message month
    pub rotate_message_table: bool,
    /// the time that the user connected.
    pub connected_at: u64,
    // Exists when we didn't register this user during HELLO
    pub deferred_user_registration: Option<UserRecord>,
}

#[derive(Clone, Default, Debug)]
pub struct CheckStorageResponse {
    /// The messages include a "topic"
    /// "topics" are messages that replace prior messages of that topic.
    /// (e.g. you can only have one message for a topic of "foo")
    pub include_topic: bool,
    /// The list of pending messages.
    pub messages: Vec<Notification>,
    /// All the messages up to this timestampl
    pub timestamp: Option<u64>,
}

/// A new endpoint has been registered. (heh, should this be a RESULT?)
pub enum RegisterResponse {
    /// Hooray! Things worked, here's your endpoint URL
    Success { endpoint: String },
    /// Crap, there was an error.
    Error { error_msg: String, status: u32 },
}

/// A user data record.
#[derive(Deserialize, PartialEq, Debug, Clone, Serialize)]
pub struct UserRecord {
    /// The UAID. This is generally a UUID4. It needs to be globally
    /// unique.
    // DynamoDB <Hash key>
    #[serde(serialize_with = "uuid_serializer")]
    pub uaid: Uuid,
    /// Time in milliseconds that the user last connected at
    pub connected_at: u64,
    /// Router type of the user
    pub router_type: String,
    /// Router-specific data
    pub router_data: Option<HashMap<String, serde_json::Value>>,
    /// Keyed time in a month the user last connected at with limited
    /// key range for indexing
    // [ed. --sigh. don't use custom timestamps kids.]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_connect: Option<u64>,
    /// Last node/port the client was or may be connected to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    /// Record version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub record_version: Option<u8>,
    /// LEGACY: Current month table in the database the user is on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_month: Option<String>,
}

impl Default for UserRecord {
    fn default() -> Self {
        let uaid = Uuid::new_v4();
        //trace!(">>> Setting default uaid: {:?}", &uaid);
        Self {
            uaid,
            connected_at: ms_since_epoch(),
            router_type: "webpush".to_string(),
            router_data: None,
            last_connect: Some(generate_last_connect()),
            node_id: None,
            record_version: Some(USER_RECORD_VERSION),
            current_month: None,
        }
    }
}

/// The outbound message record.
/// This is different that the stored `Notification`
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct NotificationRecord {
    /// The UserAgent Identifier (UAID)
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
    /// DynamoDB expiration timestamp per
    ///    https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/TTL.html
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
    /// message. Some Python code refers to this as a message_id. Endpoints generate this
    /// value before sending it to storage or a connection node.
    #[serde(skip_serializing_if = "Option::is_none")]
    updateid: Option<String>,
}

impl NotificationRecord {
    /// read the custom sort_key and convert it into something the database can use.
    fn parse_sort_key(key: &str) -> Result<RangeKey> {
        lazy_static! {
            static ref RE: RegexSet =
                RegexSet::new(&[r"^01:\S+:\S+$", r"^02:\d+:\S+$", r"^\S{3,}:\S+$",]).unwrap();
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

    // TODO: Implement as TryFrom whenever that lands
    /// Convert the
    pub fn into_notif(self) -> Result<Notification> {
        let key = Self::parse_sort_key(&self.chidmessageid)?;
        let version = key
            .legacy_version
            .or(self.updateid)
            .ok_or(ApcErrorKind::GeneralError(
                "No valid updateid/version found".into(),
            ))?;

        Ok(Notification {
            uaid: self.uaid,
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

    pub fn from_notif(uaid: &Uuid, val: Notification) -> Self {
        Self {
            uaid: *uaid,
            chidmessageid: val.sort_key(),
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
