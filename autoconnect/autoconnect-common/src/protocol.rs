//! Definition of Internal Router and Websocket protocol messages
//!
//! This module is a structured definition of several protocol. Both
//! messages received from the client and messages sent from the server are
//! defined here. The `derive(Deserialize)` and `derive(Serialize)` annotations
//! are used to generate the ability to serialize these structures to JSON,
//! using the `serde` crate. More docs for serde can be found at
//! <https://serde.rs>
use std::collections::HashMap;
use std::str::FromStr;

use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

use autopush_common::notification::Notification;

#[derive(Debug, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum BroadcastValue {
    Value(String),
    Nested(HashMap<String, BroadcastValue>),
}

#[derive(Debug, Default)]
// Used for the server to flag a webpush client to deliver a Notification or Check storage
pub enum ServerNotification {
    CheckStorage,
    Notification(Notification),
    #[default]
    Disconnect,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "messageType", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        uaid: Option<String>,
        #[serde(rename = "channelIDs", skip_serializing_if = "Option::is_none")]
        channel_ids: Option<Vec<Uuid>>,
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

    Ping,
}

impl FromStr for ClientMessage {
    type Err = serde_json::error::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // parse empty object "{}" as a Ping
        serde_json::from_str::<HashMap<(), ()>>(s)
            .map(|_| ClientMessage::Ping)
            .or_else(|_| serde_json::from_str(s))
    }
}

#[derive(Debug, Deserialize)]
pub struct ClientAck {
    #[serde(rename = "channelID")]
    pub channel_id: Uuid,
    pub version: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "messageType", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello {
        uaid: String,
        status: u32,
        // This is required for output, but will always be "true"
        use_webpush: bool,
        broadcasts: HashMap<String, BroadcastValue>,
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
        broadcasts: HashMap<String, BroadcastValue>,
    },

    Notification(Notification),

    Ping,
}

impl ServerMessage {
    pub fn to_json(&self) -> Result<String, serde_json::error::Error> {
        match self {
            // clients recognize {"messageType": "ping"} but traditionally both
            // client/server send the empty object version
            ServerMessage::Ping => Ok("{}".to_owned()),
            _ => serde_json::to_string(self),
        }
    }
}
