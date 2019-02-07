use std::collections::HashSet;
use std::fmt::{Debug, Display};
use std::rc::Rc;
use std::result::Result as StdResult;
use uuid::Uuid;

use cadence::{Counted, StatsdClient};
use chrono::Utc;
use futures::{future, Future};
use futures_backoff::retry_if;
use rusoto_dynamodb::{
    AttributeValue, BatchWriteItemError, DeleteItemError, DeleteItemInput, DeleteItemOutput,
    DynamoDb, GetItemError, GetItemInput, GetItemOutput, ListTablesInput, ListTablesOutput,
    PutItemError, PutItemInput, PutItemOutput, QueryError, QueryInput, UpdateItemError,
    UpdateItemInput, UpdateItemOutput,
};
use serde_dynamodb;

use super::models::{DynamoDbNotification, DynamoDbUser};
use super::util::generate_last_connect;
use super::{HelloResponse, MAX_EXPIRY, USER_RECORD_VERSION};
use crate::errors::*;
use crate::protocol::Notification;
use crate::util::timing::sec_since_epoch;

macro_rules! retryable_error {
    ($name:ident, $type:ty, $property:ident) => {
        pub fn $name(err: &$type) -> bool {
            match err {
                $property::InternalServerError(_) | $property::ProvisionedThroughputExceeded(_) => {
                    true
                }
                _ => false,
            }
        }
    };
}

retryable_error!(
    retryable_batchwriteitem_error,
    BatchWriteItemError,
    BatchWriteItemError
);
retryable_error!(retryable_query_error, QueryError, QueryError);
retryable_error!(retryable_delete_error, DeleteItemError, DeleteItemError);
retryable_error!(retryable_getitem_error, GetItemError, GetItemError);
retryable_error!(retryable_putitem_error, PutItemError, PutItemError);
retryable_error!(retryable_updateitem_error, UpdateItemError, UpdateItemError);

#[derive(Default)]
pub struct FetchMessageResponse {
    pub timestamp: Option<u64>,
    pub messages: Vec<Notification>,
}

/// Indicate whether this last_connect falls in the current month
fn has_connected_this_month(user: &DynamoDbUser) -> bool {
    user.last_connect.map_or(false, |v| {
        let pat = Utc::now().format("%Y%m").to_string();
        v.to_string().starts_with(&pat)
    })
}

/// A blocking list_tables call only called during initialization
/// (prior to an any active tokio executor)
pub fn list_tables_sync(
    ddb: Rc<Box<dyn DynamoDb>>,
    start_key: Option<String>,
) -> Result<ListTablesOutput> {
    let input = ListTablesInput {
        exclusive_start_table_name: start_key,
        limit: Some(100),
    };
    ddb.list_tables(input)
        .sync()
        .chain_err(|| "Unable to list tables")
}

pub fn fetch_messages(
    ddb: Rc<Box<dyn DynamoDb>>,
    metrics: &Rc<StatsdClient>,
    table_name: &str,
    uaid: &Uuid,
    limit: u32,
) -> impl Future<Item = FetchMessageResponse, Error = Error> {
    let attr_values = hashmap! {
        ":uaid".to_string() => val!(S => uaid.to_simple().to_string()),
        ":cmi".to_string() => val!(S => "02"),
    };
    let input = QueryInput {
        key_condition_expression: Some("uaid = :uaid AND chidmessageid < :cmi".to_string()),
        expression_attribute_values: Some(attr_values),
        table_name: table_name.to_string(),
        consistent_read: Some(true),
        limit: Some(limit as i64),
        ..Default::default()
    };

    let metrics = Rc::clone(metrics);
    retry_if(move || ddb.query(input.clone()), retryable_query_error)
        .chain_err(|| ErrorKind::MessageFetch)
        .and_then(move |output| {
            let mut notifs: Vec<DynamoDbNotification> =
                output.items.map_or_else(Vec::new, |items| {
                    debug!("Got response of: {:?}", items);
                    items
                        .into_iter()
                        .inspect(|i| debug!("Item: {:?}", i))
                        .filter_map(|item| {
                            let item2 = item.clone();
                            ok_or_inspect(serde_dynamodb::from_hashmap(item), |e| {
                                conversion_err(&metrics, e, item2, "serde_dynamodb_from_hashmap")
                            })
                        })
                        .collect()
                });
            if notifs.is_empty() {
                return Ok(Default::default());
            }

            // Load the current_timestamp from the subscription registry entry which is
            // the first DynamoDbNotification and remove it from the vec.
            let timestamp = notifs.remove(0).current_timestamp;
            // Convert any remaining DynamoDbNotifications to Notification's
            let messages = notifs
                .into_iter()
                .filter_map(|ddb_notif| {
                    let ddb_notif2 = ddb_notif.clone();
                    ok_or_inspect(ddb_notif.into_notif(), |e| {
                        conversion_err(&metrics, e, ddb_notif2, "into_notif")
                    })
                })
                .collect();
            Ok(FetchMessageResponse {
                timestamp,
                messages,
            })
        })
}

pub fn fetch_timestamp_messages(
    ddb: Rc<Box<dyn DynamoDb>>,
    metrics: &Rc<StatsdClient>,
    table_name: &str,
    uaid: &Uuid,
    timestamp: Option<u64>,
    limit: u32,
) -> impl Future<Item = FetchMessageResponse, Error = Error> {
    let range_key = if let Some(ts) = timestamp {
        format!("02:{}:z", ts)
    } else {
        "01;".to_string()
    };
    let attr_values = hashmap! {
        ":uaid".to_string() => val!(S => uaid.to_simple().to_string()),
        ":cmi".to_string() => val!(S => range_key),
    };
    let input = QueryInput {
        key_condition_expression: Some("uaid = :uaid AND chidmessageid > :cmi".to_string()),
        expression_attribute_values: Some(attr_values),
        table_name: table_name.to_string(),
        consistent_read: Some(true),
        limit: Some(limit as i64),
        ..Default::default()
    };

    let metrics = Rc::clone(metrics);
    retry_if(move || ddb.query(input.clone()), retryable_query_error)
        .chain_err(|| ErrorKind::MessageFetch)
        .and_then(move |output| {
            let messages = output.items.map_or_else(Vec::new, |items| {
                debug!("Got response of: {:?}", items);
                items
                    .into_iter()
                    .filter_map(|item| {
                        let item2 = item.clone();
                        ok_or_inspect(serde_dynamodb::from_hashmap(item), |e| {
                            conversion_err(&metrics, e, item2, "serde_dynamodb_from_hashmap")
                        })
                    })
                    .filter_map(|ddb_notif: DynamoDbNotification| {
                        let ddb_notif2 = ddb_notif.clone();
                        ok_or_inspect(ddb_notif.into_notif(), |e| {
                            conversion_err(&metrics, e, ddb_notif2, "into_notif")
                        })
                    })
                    .collect()
            });
            let timestamp = messages.iter().filter_map(|m| m.sortkey_timestamp).max();
            Ok(FetchMessageResponse {
                timestamp,
                messages,
            })
        })
}

pub fn drop_user(
    ddb: Rc<Box<dyn DynamoDb>>,
    uaid: &Uuid,
    router_table_name: &str,
) -> impl Future<Item = DeleteItemOutput, Error = Error> {
    let input = DeleteItemInput {
        table_name: router_table_name.to_string(),
        key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
        ..Default::default()
    };
    retry_if(
        move || ddb.delete_item(input.clone()),
        retryable_delete_error,
    )
    .chain_err(|| "Error dropping user")
}

pub fn get_uaid(
    ddb: Rc<Box<dyn DynamoDb>>,
    uaid: &Uuid,
    router_table_name: &str,
) -> impl Future<Item = GetItemOutput, Error = Error> {
    let input = GetItemInput {
        table_name: router_table_name.to_string(),
        consistent_read: Some(true),
        key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
        ..Default::default()
    };
    retry_if(move || ddb.get_item(input.clone()), retryable_getitem_error)
        .chain_err(|| "Error fetching user")
}

pub fn register_user(
    ddb: Rc<Box<dyn DynamoDb>>,
    user: &DynamoDbUser,
    router_table: &str,
) -> impl Future<Item = PutItemOutput, Error = Error> {
    let item = match serde_dynamodb::to_hashmap(user) {
        Ok(item) => item,
        Err(e) => return future::err(e).chain_err(|| "Failed to serialize item"),
    };
    let router_table = router_table.to_string();
    let attr_values = hashmap! {
        ":router_type".to_string() => val!(S => user.router_type),
        ":connected_at".to_string() => val!(N => user.connected_at),
    };

    retry_if(
        move || {
            debug!("Registering user: {:?}", item);
            ddb.put_item(PutItemInput {
                item: item.clone(),
                table_name: router_table.clone(),
                expression_attribute_values: Some(attr_values.clone()),
                condition_expression: Some(
                    r#"(
                            attribute_not_exists(router_type) or
                            (router_type = :router_type)
                        ) and (
                            attribute_not_exists(node_id) or
                            (connected_at < :connected_at)
                        )"#
                    .to_string(),
                ),
                return_values: Some("ALL_OLD".to_string()),
                ..Default::default()
            })
        },
        retryable_putitem_error,
    )
    .chain_err(|| "Error storing user record")
}

pub fn update_user_message_month(
    ddb: Rc<Box<dyn DynamoDb>>,
    uaid: &Uuid,
    router_table_name: &str,
    message_month: &str,
) -> impl Future<Item = (), Error = Error> {
    let attr_values = hashmap! {
        ":curmonth".to_string() => val!(S => message_month.to_string()),
        ":lastconnect".to_string() => val!(N => generate_last_connect().to_string()),
    };
    let update_item = UpdateItemInput {
        key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
        update_expression: Some(
            "SET current_month=:curmonth, last_connect=:lastconnect".to_string(),
        ),
        expression_attribute_values: Some(attr_values),
        table_name: router_table_name.to_string(),
        ..Default::default()
    };

    retry_if(
        move || {
            ddb.update_item(update_item.clone())
                .and_then(|_| future::ok(()))
        },
        retryable_updateitem_error,
    )
    .chain_err(|| "Error updating user message month")
}

pub fn all_channels(
    ddb: Rc<Box<dyn DynamoDb>>,
    uaid: &Uuid,
    message_table_name: &str,
) -> impl Future<Item = HashSet<String>, Error = Error> {
    let input = GetItemInput {
        table_name: message_table_name.to_string(),
        consistent_read: Some(true),
        key: ddb_item! {
            uaid: s => uaid.to_simple().to_string(),
            chidmessageid: s => " ".to_string()
        },
        ..Default::default()
    };

    retry_if(move || ddb.get_item(input.clone()), retryable_getitem_error)
        .and_then(|output| {
            let channels = output
                .item
                .and_then(|item| {
                    serde_dynamodb::from_hashmap(item)
                        .ok()
                        .and_then(|notif: DynamoDbNotification| notif.chids)
                })
                .unwrap_or_else(HashSet::new);
            future::ok(channels)
        })
        .or_else(|_err| future::ok(HashSet::new()))
}

pub fn save_channels(
    ddb: Rc<Box<dyn DynamoDb>>,
    uaid: &Uuid,
    channels: HashSet<String>,
    message_table_name: &str,
) -> impl Future<Item = (), Error = Error> {
    let chids: Vec<String> = channels.into_iter().collect();
    let expiry = sec_since_epoch() + 2 * MAX_EXPIRY;
    let attr_values = hashmap! {
        ":chids".to_string() => val!(SS => chids),
        ":expiry".to_string() => val!(N => expiry),
    };
    let update_item = UpdateItemInput {
        key: ddb_item! {
            uaid: s => uaid.to_simple().to_string(),
            chidmessageid: s => " ".to_string()
        },
        update_expression: Some("ADD chids :chids SET expiry=:expiry".to_string()),
        expression_attribute_values: Some(attr_values),
        table_name: message_table_name.to_string(),
        ..Default::default()
    };

    retry_if(
        move || {
            ddb.update_item(update_item.clone())
                .and_then(|_| future::ok(()))
        },
        retryable_updateitem_error,
    )
    .chain_err(|| "Error saving channels")
}

pub fn unregister_channel_id(
    ddb: Rc<Box<dyn DynamoDb>>,
    uaid: &Uuid,
    channel_id: &Uuid,
    message_table_name: &str,
) -> impl Future<Item = UpdateItemOutput, Error = Error> {
    let chid = channel_id.to_hyphenated().to_string();
    let attr_values = hashmap! {
        ":channel_id".to_string() => val!(SS => vec![chid]),
    };
    let update_item = UpdateItemInput {
        key: ddb_item! {
            uaid: s => uaid.to_simple().to_string(),
            chidmessageid: s => " ".to_string()
        },
        update_expression: Some("DELETE chids :channel_id".to_string()),
        expression_attribute_values: Some(attr_values),
        table_name: message_table_name.to_string(),
        ..Default::default()
    };

    retry_if(
        move || ddb.update_item(update_item.clone()),
        retryable_updateitem_error,
    )
    .chain_err(|| "Error unregistering channel")
}

pub fn lookup_user(
    ddb: Rc<Box<dyn DynamoDb>>,
    metrics: &Rc<StatsdClient>,
    uaid: &Uuid,
    connected_at: &u64,
    router_url: &str,
    router_table_name: &str,
    message_table_names: &[String],
    current_message_month: &str,
) -> MyFuture<(HelloResponse, Option<DynamoDbUser>)> {
    let response = get_uaid(ddb.clone(), uaid, router_table_name);
    // Prep all these for the move into the static closure capture
    let cur_month = current_message_month.to_string();
    let uaid2 = *uaid;
    let router_table = router_table_name.to_string();
    let messages_tables = message_table_names.to_vec();
    let connected_at = *connected_at;
    let router_url = router_url.to_string();
    let metrics = Rc::clone(metrics);
    let response = response.and_then(move |data| -> MyFuture<_> {
        let mut hello_response = HelloResponse {
            message_month: cur_month.clone(),
            connected_at,
            ..Default::default()
        };
        let user = handle_user_result(
            &cur_month,
            &messages_tables,
            connected_at,
            router_url,
            data,
            &mut hello_response,
        );
        match user {
            Ok(user) => Box::new(future::ok((hello_response, Some(user)))),
            Err((false, _)) => Box::new(future::ok((hello_response, None))),
            Err((true, code)) => {
                metrics
                    .incr_with_tags("ua.expiration")
                    .with_tag("code", &code.to_string())
                    .send();
                let response = drop_user(ddb, &uaid2, &router_table)
                    .and_then(|_| future::ok((hello_response, None)))
                    .chain_err(|| "Unable to drop user");
                Box::new(response)
            }
        }
    });
    Box::new(response)
}

/// Helper function for determining if a returned user record is valid for use
/// or if it should be dropped and a new one created.
fn handle_user_result(
    cur_month: &String,
    messages_tables: &[String],
    connected_at: u64,
    router_url: String,
    data: GetItemOutput,
    hello_response: &mut HelloResponse,
) -> StdResult<DynamoDbUser, (bool, u16)> {
    let item = data.item.ok_or((false, 104))?;
    let mut user: DynamoDbUser = serde_dynamodb::from_hashmap(item).map_err(|_| (true, 104))?;

    let user_month = user.current_month.clone().ok_or((true, 104))?;
    if !messages_tables.contains(&user_month) {
        return Err((true, 105));
    }
    hello_response.check_storage = true;
    hello_response.rotate_message_table = user_month != *cur_month;
    hello_response.message_month = user_month;
    hello_response.reset_uaid = user
        .record_version
        .map_or(true, |rec_ver| rec_ver < USER_RECORD_VERSION);

    user.last_connect = if has_connected_this_month(&user) {
        None
    } else {
        Some(generate_last_connect())
    };
    user.node_id = Some(router_url);
    user.connected_at = connected_at;
    Ok(user)
}

/// Like Result::ok, convert from Result<T, E> to Option<T> but applying a
/// function to the Err value
fn ok_or_inspect<T, E, F>(result: StdResult<T, E>, op: F) -> Option<T>
where
    F: FnOnce(E),
{
    match result {
        Ok(t) => Some(t),
        Err(e) => {
            op(e);
            None
        }
    }
}

/// Log/metric errors during conversions to Notification
fn conversion_err<E, F>(metrics: &StatsdClient, err: E, item: F, name: &'static str)
where
    E: Display,
    F: Debug,
{
    error!("Failed {}, item: {:?}, conversion: {}", name, item, err);
    metrics
        .incr_with_tags("ua.notification_read.error")
        .with_tag("conversion", name)
        .send();
}
