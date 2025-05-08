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
use strum_macros::{AsRefStr, Display, EnumString};
use uuid::Uuid;

use autopush_common::notification::Notification;

/// Message types for WebPush protocol messages.
///
/// This enum should be used instead of string literals when referring to message types.
/// String serialization is handled automatically via the strum traits.
///
/// Example:
/// ```
///  use autoconnect_common::protocol::MessageType;
///
/// let message_type = MessageType::Hello;
/// let message_str = message_type.as_str();  // Returns "hello"
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, AsRefStr, Display, EnumString)]
#[strum(serialize_all = "snake_case")]
pub enum MessageType {
    Hello,
    Register,
    Unregister,
    BroadcastSubscribe,
    Ack,
    Nack,
    Ping,
    Notification,
    Broadcast,
}

impl MessageType {
    /// Converts the enum to its string representation
    pub fn as_str(&self) -> &str {
        self.as_ref()
    }

    /// Returns the expected message type string for error messages
    pub fn expected_msg(&self) -> String {
        format!(r#"Expected messageType="{}""#, self.as_str())
    }
}

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
        _channel_ids: Option<Vec<Uuid>>,
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

impl ClientMessage {
    /// Get the message type of this message
    pub fn message_type(&self) -> MessageType {
        match self {
            ClientMessage::Hello { .. } => MessageType::Hello,
            ClientMessage::Register { .. } => MessageType::Register,
            ClientMessage::Unregister { .. } => MessageType::Unregister,
            ClientMessage::BroadcastSubscribe { .. } => MessageType::BroadcastSubscribe,
            ClientMessage::Ack { .. } => MessageType::Ack,
            ClientMessage::Nack { .. } => MessageType::Nack,
            ClientMessage::Ping => MessageType::Ping,
        }
    }
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

/// Returned ACKnowledgement of the received message by the User Agent.
/// This is the payload for the `messageType:ack` packet.
///
#[derive(Debug, Deserialize)]
pub struct ClientAck {
    /// The channel_id which received messages
    #[serde(rename = "channelID")]
    pub channel_id: Uuid,
    /// The corresponding version number for the message.
    pub version: String,
    /// An optional code categorizing the status of the ACK
    pub code: Option<u16>,
}

impl ClientAck {
    #[cfg(feature = "reliable_report")]
    pub fn reliability_state(&self) -> autopush_common::reliability::ReliabilityState {
        match self.code.unwrap_or(100) {
            101 => autopush_common::reliability::ReliabilityState::DecryptionError,
            102 => autopush_common::reliability::ReliabilityState::NotDelivered,
            // 100 (ignore/treat anything else as 100)
            _ => autopush_common::reliability::ReliabilityState::Delivered,
        }
    }
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
    /// Get the message type of this message
    pub fn message_type(&self) -> MessageType {
        match self {
            ServerMessage::Hello { .. } => MessageType::Hello,
            ServerMessage::Register { .. } => MessageType::Register,
            ServerMessage::Unregister { .. } => MessageType::Unregister,
            ServerMessage::Broadcast { .. } => MessageType::Broadcast,
            ServerMessage::Notification(..) => MessageType::Notification,
            ServerMessage::Ping => MessageType::Ping,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::error::Error> {
        match self {
            // Traditionally both client/server send the empty object version for ping
            ServerMessage::Ping => Ok("{}".to_string()),
            _ => serde_json::to_string(self),
        }
    }
}
