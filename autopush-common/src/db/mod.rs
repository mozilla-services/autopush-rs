use std::collections::HashSet;
use std::env;
use std::rc::Rc;
use uuid::Uuid;

use cadence::StatsdClient;
use futures::{future, Future};
use futures_backoff::retry_if;
use rusoto_core::{HttpClient, Region};
use rusoto_credential::StaticProvider;
use rusoto_dynamodb::{
    AttributeValue, BatchWriteItemInput, DeleteItemInput, DynamoDb, DynamoDbClient, PutRequest,
    UpdateItemInput, UpdateItemOutput, WriteRequest,
};
use serde_dynamodb;

#[macro_use]
mod macros;
mod commands;
mod models;
mod util;

use crate::errors::*;
use crate::notification::Notification;
use crate::util::timing::sec_since_epoch;

use self::commands::{
    retryable_batchwriteitem_error, retryable_delete_error, retryable_updateitem_error,
    FetchMessageResponse,
};
use self::models::{DynamoDbNotification, DynamoDbUser};

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

pub struct DynamoStorage {
    ddb: Rc<Box<dyn DynamoDb>>,
    metrics: Rc<StatsdClient>,
    router_table_name: String,
    message_table_names: Vec<String>,
    pub current_message_month: String,
}

impl DynamoStorage {
    pub fn from_opts(
        message_table_name: &str,
        router_table_name: &str,
        metrics: StatsdClient,
    ) -> Result<Self> {
        let ddb: Box<dyn DynamoDb> = if let Ok(endpoint) = env::var("AWS_LOCAL_DYNAMODB") {
            Box::new(DynamoDbClient::new_with(
                HttpClient::new().chain_err(|| "TLS initialization error")?,
                StaticProvider::new_minimal("BogusKey".to_string(), "BogusKey".to_string()),
                Region::Custom {
                    name: "us-east-1".to_string(),
                    endpoint,
                },
            ))
        } else {
            Box::new(DynamoDbClient::new(Region::default()))
        };
        let ddb = Rc::new(ddb);

        let mut message_table_names = list_message_tables(&ddb, &message_table_name)
            .map_err(|_| "Failed to locate message tables")?;
        // Valid message months are the current and last 2 months
        message_table_names.sort_unstable_by(|a, b| b.cmp(a));
        message_table_names.truncate(3);
        message_table_names.reverse();
        let current_message_month = message_table_names
            .last()
            .ok_or("No last message month found")?
            .to_string();

        Ok(Self {
            ddb,
            metrics: Rc::new(metrics),
            router_table_name: router_table_name.to_owned(),
            message_table_names,
            current_message_month,
        })
    }

    pub fn increment_storage(
        &self,
        table_name: &str,
        uaid: &Uuid,
        timestamp: &str,
    ) -> impl Future<Item = UpdateItemOutput, Error = Error> {
        let ddb = self.ddb.clone();
        let expiry = sec_since_epoch() + 2 * MAX_EXPIRY;
        let attr_values = hashmap! {
            ":timestamp".to_string() => val!(N => timestamp),
            ":expiry".to_string() => val!(N => expiry),
        };
        let update_input = UpdateItemInput {
            key: ddb_item! {
                uaid: s => uaid.to_simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            update_expression: Some("SET current_timestamp=:timestamp, expiry=:expiry".to_string()),
            expression_attribute_values: Some(attr_values),
            table_name: table_name.to_string(),
            ..Default::default()
        };

        retry_if(
            move || ddb.update_item(update_input.clone()),
            retryable_updateitem_error,
        )
        .chain_err(|| "Error incrementing storage")
    }

    pub fn hello(
        &self,
        connected_at: &u64,
        uaid: Option<&Uuid>,
        router_url: &str,
    ) -> impl Future<Item = HelloResponse, Error = Error> {
        let response: MyFuture<(HelloResponse, Option<DynamoDbUser>)> = if let Some(uaid) = uaid {
            commands::lookup_user(
                self.ddb.clone(),
                &self.metrics,
                &uaid,
                connected_at,
                router_url,
                &self.router_table_name,
                &self.message_table_names,
                &self.current_message_month,
            )
        } else {
            Box::new(future::ok((
                HelloResponse {
                    message_month: self.current_message_month.clone(),
                    connected_at: *connected_at,
                    ..Default::default()
                },
                None,
            )))
        };
        let ddb = self.ddb.clone();
        let router_url = router_url.to_string();
        let router_table_name = self.router_table_name.clone();
        let connected_at = *connected_at;

        response.and_then(move |(mut hello_response, user_opt)| {
            let hello_message_month = hello_response.message_month.clone();
            let user = user_opt.unwrap_or_else(|| DynamoDbUser {
                current_month: Some(hello_message_month),
                node_id: Some(router_url),
                connected_at,
                ..Default::default()
            });
            let uaid = user.uaid;
            let mut err_response = hello_response.clone();
            err_response.connected_at = connected_at;
            commands::register_user(ddb, &user, &router_table_name)
                .and_then(move |result| {
                    debug!("Success adding user, item output: {:?}", result);
                    hello_response.uaid = Some(uaid);
                    future::ok(hello_response)
                })
                .or_else(move |e| {
                    debug!("Error registering user: {:?}", e);
                    future::ok(err_response)
                })
        })
    }

    pub fn register(
        &self,
        uaid: &Uuid,
        channel_id: &Uuid,
        message_month: &str,
        endpoint: &str,
    ) -> MyFuture<RegisterResponse> {
        let ddb = self.ddb.clone();
        let mut chids = HashSet::new();
        let endpoint = endpoint.to_owned();
        chids.insert(channel_id.to_hyphenated().to_string());
        let response = commands::save_channels(ddb, uaid, chids, message_month)
            .and_then(move |_| future::ok(RegisterResponse::Success { endpoint }))
            .or_else(move |_| {
                future::ok(RegisterResponse::Error {
                    status: 503,
                    error_msg: "Failed to register channel".to_string(),
                })
            });
        Box::new(response)
    }

    pub fn drop_uaid(&self, uaid: &Uuid) -> impl Future<Item = (), Error = Error> {
        commands::drop_user(self.ddb.clone(), uaid, &self.router_table_name)
            .and_then(|_| future::ok(()))
            .chain_err(|| "Unable to drop user record")
    }

    pub fn unregister(
        &self,
        uaid: &Uuid,
        channel_id: &Uuid,
        message_month: &str,
    ) -> impl Future<Item = bool, Error = Error> {
        commands::unregister_channel_id(self.ddb.clone(), uaid, channel_id, message_month)
            .and_then(|_| future::ok(true))
            .or_else(|_| future::ok(false))
    }

    /// Migrate a user to a new month table
    pub fn migrate_user(
        &self,
        uaid: &Uuid,
        message_month: &str,
    ) -> impl Future<Item = (), Error = Error> {
        let uaid = *uaid;
        let ddb = self.ddb.clone();
        let ddb2 = self.ddb.clone();
        let cur_month = self.current_message_month.to_string();
        let cur_month2 = cur_month.clone();
        let router_table_name = self.router_table_name.clone();

        commands::all_channels(self.ddb.clone(), &uaid, message_month)
            .and_then(move |channels| -> MyFuture<_> {
                if channels.is_empty() {
                    Box::new(future::ok(()))
                } else {
                    Box::new(commands::save_channels(ddb, &uaid, channels, &cur_month))
                }
            })
            .and_then(move |_| {
                commands::update_user_message_month(ddb2, &uaid, &router_table_name, &cur_month2)
            })
            .and_then(|_| future::ok(()))
            .chain_err(|| "Unable to migrate user")
    }

    /// Store a batch of messages when shutting down
    pub fn store_messages(
        &self,
        uaid: &Uuid,
        message_month: &str,
        messages: Vec<Notification>,
    ) -> impl Future<Item = (), Error = Error> {
        let ddb = self.ddb.clone();
        let put_items: Vec<WriteRequest> = messages
            .into_iter()
            .filter_map(|n| {
                serde_dynamodb::to_hashmap(&DynamoDbNotification::from_notif(uaid, n))
                    .ok()
                    .map(|hm| WriteRequest {
                        put_request: Some(PutRequest { item: hm }),
                        delete_request: None,
                    })
            })
            .collect();
        let batch_input = BatchWriteItemInput {
            request_items: hashmap! { message_month.to_string() => put_items },
            ..Default::default()
        };

        retry_if(
            move || ddb.batch_write_item(batch_input.clone()),
            retryable_batchwriteitem_error,
        )
        .and_then(|_| future::ok(()))
        .map_err(|err| {
            debug!("Error saving notification: {:?}", err);
            err
        })
        // TODO: Use Sentry to capture/report this error
        .chain_err(|| "Error saving notifications")
    }

    /// Delete a given notification from the database
    ///
    /// No checks are done to see that this message came from the database or has
    /// sufficient properties for a delete as that is expected to have been done
    /// before this is called.
    pub fn delete_message(
        &self,
        table_name: &str,
        uaid: &Uuid,
        notif: &Notification,
    ) -> impl Future<Item = (), Error = Error> {
        let ddb = self.ddb.clone();
        let delete_input = DeleteItemInput {
            table_name: table_name.to_string(),
            key: ddb_item! {
               uaid: s => uaid.to_simple().to_string(),
               chidmessageid: s => notif.sort_key()
            },
            ..Default::default()
        };

        retry_if(
            move || ddb.delete_item(delete_input.clone()),
            retryable_delete_error,
        )
        .and_then(|_| future::ok(()))
        .chain_err(|| "Error deleting notification")
    }

    pub fn check_storage(
        &self,
        table_name: &str,
        uaid: &Uuid,
        include_topic: bool,
        timestamp: Option<u64>,
    ) -> impl Future<Item = CheckStorageResponse, Error = Error> {
        let response: MyFuture<FetchMessageResponse> = if include_topic {
            Box::new(commands::fetch_messages(
                self.ddb.clone(),
                &self.metrics,
                table_name,
                uaid,
                11 as u32,
            ))
        } else {
            Box::new(future::ok(Default::default()))
        };
        let uaid = *uaid;
        let table_name = table_name.to_string();
        let ddb = self.ddb.clone();
        let metrics = Rc::clone(&self.metrics);

        response.and_then(move |resp| -> MyFuture<_> {
            // Return now from this future if we have messages
            if !resp.messages.is_empty() {
                debug!("Topic message returns: {:?}", resp.messages);
                return Box::new(future::ok(CheckStorageResponse {
                    include_topic: true,
                    messages: resp.messages,
                    timestamp: resp.timestamp,
                }));
            }
            // Use the timestamp returned by the topic query if we were looking at the topics
            let timestamp = if include_topic {
                resp.timestamp
            } else {
                timestamp
            };
            let next_query: MyFuture<_> = {
                if resp.messages.is_empty() || resp.timestamp.is_some() {
                    Box::new(commands::fetch_timestamp_messages(
                        ddb,
                        &metrics,
                        table_name.as_ref(),
                        &uaid,
                        timestamp,
                        10 as u32,
                    ))
                } else {
                    Box::new(future::ok(Default::default()))
                }
            };
            let next_query = next_query.and_then(move |resp: FetchMessageResponse| {
                // If we didn't get a timestamp off the last query, use the original
                // value if passed one
                let timestamp = resp.timestamp.or(timestamp);
                Ok(CheckStorageResponse {
                    include_topic: false,
                    messages: resp.messages,
                    timestamp,
                })
            });
            Box::new(next_query)
        })
    }

    pub fn get_user(&self, uaid: &Uuid) -> impl Future<Item = DynamoDbUser, Error = Error> {
        let ddb = self.ddb.clone();
        let result = commands::get_uaid(ddb, uaid, &self.router_table_name).and_then(|result| {
            future::result(
                result
                    .item
                    .ok_or_else(|| "No user record found".into())
                    .and_then(|item| {
                        let user = serde_dynamodb::from_hashmap(item);
                        user.chain_err(|| "Error deserializing")
                    }),
            )
        });
        Box::new(result)
    }
}

pub fn list_message_tables(ddb: &Rc<Box<dyn DynamoDb>>, prefix: &str) -> Result<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    let mut start_key = None;
    loop {
        let result = commands::list_tables_sync(Rc::clone(ddb), start_key)?;
        start_key = result.last_evaluated_table_name;
        if let Some(table_names) = result.table_names {
            names.extend(table_names);
        }
        if start_key.is_none() {
            break;
        }
    }
    let names = names
        .into_iter()
        .filter(|name| name.starts_with(prefix))
        .collect();
    Ok(names)
}
