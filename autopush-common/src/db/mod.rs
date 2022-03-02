use std::collections::HashSet;
use std::env;
use uuid::Uuid;

use cadence::StatsdClient;
use futures::{future, Future};
use futures_backoff::retry_if;

pub mod dynamodb;
pub mod models;
mod util;

use crate::errors::*;
use crate::notification::Notification;
use crate::util::timing::sec_since_epoch;

use self::dynamodb::commands::{
    retryable_batchwriteitem_error, retryable_delete_error, retryable_putitem_error,
    retryable_updateitem_error, FetchMessageResponse,
};

const MAX_EXPIRY: u64 = 2_592_000;
const USER_RECORD_VERSION: u8 = 1;

/// Basic requirements for notification content to deliver to websocket client
///  - channelID  (the subscription website intended for)
///  - version    (only really utilized for notification acknowledgement in
///                webpush, used to be the sole carrier of data, can now be anything)
///  - data       (encrypted content)
///  - headers    (hash of crypto headers: encoding, encrypption, crypto-key, encryption-key)
#[derive(Default, Clone)]
pub struct HelloResponse {
    pub uaid: Option<Uuid>,
    pub message_month: String,
    pub check_storage: bool,
    pub reset_uaid: bool,
    pub rotate_message_table: bool,
    pub connected_at: u64,
    // Exists when we didn't register this user during HELLO
    pub deferred_user_registration: Option<dynamodb::DynamoDbUser>,
}

pub struct CheckStorageResponse {
    pub include_topic: bool,
    pub messages: Vec<Notification>,
    pub timestamp: Option<u64>,
}

pub enum RegisterResponse {
    Success { endpoint: String },
    Error { error_msg: String, status: u32 },
}


