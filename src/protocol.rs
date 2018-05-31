//! Definition of Internal Router, Python, and Websocket protocol messages
//!
//! This module is a structured definition of several protocol. Both
//! messages received from the client and messages sent from the server are
//! defined here. The `derive(Deserialize)` and `derive(Serialize)` annotations
//! are used to generate the ability to serialize these structures to JSON,
//! using the `serde` crate. More docs for serde can be found at
//! https://serde.rs

use std::collections::HashMap;
use uuid::Uuid;

use util::ms_since_epoch;

// Used for the server to flag a webpush client to deliver a Notification or Check storage
pub enum ServerNotification {
    CheckStorage,
    Notification(Notification),
    Disconnect,
}

impl Default for ServerNotification {
    fn default() -> Self {
        ServerNotification::Disconnect
    }
}

#[derive(Deserialize)]
#[serde(tag = "messageType", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        uaid: Option<String>,
        #[serde(rename = "channelIDs", skip_serializing_if = "Option::is_none")]
        channel_ids: Option<Vec<Uuid>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        use_webpush: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        broadcasts: Option<HashMap<String, String>>,
    },

    Register {
        #[serde(rename = "channelID")]
        channel_id: String,
        key: Option<String>,
    },

    Unregister {
        #[serde(rename = "channelID")]
        channel_id: Uuid,
        code: Option<u32>,
    },

    BroadcastSubscribe {
        broadcasts: HashMap<String, String>,
    },

    Ack {
        updates: Vec<ClientAck>,
    },

    Nack {
        code: Option<i32>,
        version: String,
    },
}

#[derive(Deserialize)]
pub struct ClientAck {
    #[serde(rename = "channelID")]
    pub channel_id: Uuid,
    pub version: String,
}

#[derive(Serialize)]
#[serde(tag = "messageType", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        uaid: String,
        status: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        use_webpush: Option<bool>,
        broadcasts: HashMap<String, String>,
    },

    Register {
        #[serde(rename = "channelID")]
        channel_id: Uuid,
        status: u32,
        #[serde(rename = "pushEndpoint")]
        push_endpoint: String,
    },

    Unregister {
        #[serde(rename = "channelID")]
        channel_id: Uuid,
        status: u32,
    },

    Broadcast {
        broadcasts: HashMap<String, String>,
    },

    Notification(Notification),
}

#[derive(Serialize, Default, Deserialize, Clone, Debug)]
pub struct Notification {
    #[serde(rename = "channelID")]
    pub channel_id: Uuid,
    pub version: String,
    #[serde(default = "default_ttl", skip_serializing)]
    pub ttl: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    pub timestamp: u32,
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
}

fn default_ttl() -> u32 {
    0
}
