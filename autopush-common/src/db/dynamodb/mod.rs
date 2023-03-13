use std::collections::HashSet;
use std::env;
use std::fmt::{Debug, Display};
use std::result::Result as StdResult;
use std::sync::Arc;

use crate::db::client::DbClient;
use crate::db::dynamodb::retry::{
    retry_policy, retryable_delete_error, retryable_describe_table_error, retryable_getitem_error,
    retryable_putitem_error, retryable_updateitem_error,
};
use crate::db::error::{DbError, DbResult};
use crate::db::{
    client::FetchMessageResponse, DbSettings, NotificationRecord, UserRecord, MAX_CHANNEL_TTL,
    MAX_EXPIRY,
};
use crate::notification::Notification;
use crate::util::sec_since_epoch;

use async_trait::async_trait;
// use crate::db::dynamodb::{ddb_item, hashmap, val};
use cadence::{CountedExt, StatsdClient};
use chrono::Utc;
use rusoto_core::credential::StaticProvider;
use rusoto_core::{HttpClient, Region, RusotoError};
use rusoto_dynamodb::{
    AttributeValue, DeleteItemInput, DescribeTableError, DescribeTableInput, DynamoDb,
    DynamoDbClient, GetItemInput, PutItemInput, QueryInput, UpdateItemInput,
};
use serde::Deserialize;
use uuid::Uuid;

use super::util::generate_last_connect;
use super::{HelloResponse, USER_RECORD_VERSION};

#[macro_use]
pub mod macros;
pub mod commands;
pub mod retry;

#[derive(Clone, Debug, Deserialize)]
pub struct DynamoDbSettings {
    pub router_table: String,
    pub message_table: String,
    pub message_table_names: Vec<String>,
    pub current_message_month: String,
}

impl Default for DynamoDbSettings {
    fn default() -> Self {
        Self {
            router_table: "router".to_string(),
            message_table: "message".to_string(),
            message_table_names: Vec::new(),
            current_message_month: String::default(),
        }
    }
}

impl TryFrom<&str> for DynamoDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(setting_string)
            .map_err(|e| DbError::General(format!("Could not parse DdbSettings: {:?}", e)))
    }
}

#[derive(Clone)]
pub struct DdbClientImpl {
    db_client: DynamoDbClient,
    metrics: Arc<StatsdClient>,
    settings: DynamoDbSettings,
}

impl DdbClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        let ddb = if let Ok(endpoint) = env::var("AWS_LOCAL_DYNAMODB") {
            DynamoDbClient::new_with(
                HttpClient::new().expect("TLS initialization error"),
                StaticProvider::new_minimal("BogusKey".to_string(), "BogusKey".to_string()),
                Region::Custom {
                    name: "us-east-1".to_string(),
                    endpoint,
                },
            )
        } else {
            DynamoDbClient::new(Region::default())
        };

        let settings =
            DynamoDbSettings::try_from(settings.db_settings.as_ref()).unwrap_or_default();

        Ok(Self {
            db_client: ddb,
            metrics,
            settings,
        })
    }

    /// Check if a table exists
    async fn table_exists(&self, table_name: String) -> DbResult<bool> {
        let input = DescribeTableInput { table_name };

        let output = match retry_policy()
            .retry_if(
                || self.db_client.describe_table(input.clone()),
                retryable_describe_table_error(self.metrics.clone()),
            )
            .await
        {
            Ok(output) => output,
            Err(RusotoError::Service(DescribeTableError::ResourceNotFound(_))) => {
                return Ok(false);
            }
            Err(e) => return Err(e.into()),
        };

        let status = output
            .table
            .and_then(|table| table.table_status)
            .ok_or(DbError::TableStatusUnknown)?
            .to_uppercase();

        Ok(["CREATING", "UPDATING", "ACTIVE"].contains(&status.as_str()))
    }
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

#[allow(clippy::field_reassign_with_default)]
#[async_trait]
impl DbClient for DdbClientImpl {
    async fn add_user(&self, user: &UserRecord) -> DbResult<()> {
        let input = PutItemInput {
            table_name: self.settings.router_table.clone(),
            item: serde_dynamodb::to_hashmap(user)?,
            condition_expression: Some("attribute_not_exists(uaid)".to_string()),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.put_item(input.clone()),
                retryable_putitem_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn update_user(&self, user: &UserRecord) -> DbResult<()> {
        let mut user_map = serde_dynamodb::to_hashmap(&user)?;
        user_map.remove("uaid");
        let input = UpdateItemInput {
            table_name: self.settings.router_table.clone(),
            key: ddb_item! { uaid: s => user.uaid.simple().to_string() },
            update_expression: Some(format!(
                "SET {}",
                user_map
                    .keys()
                    .map(|key| format!("{0}=:{0}", key))
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            expression_attribute_values: Some(
                user_map
                    .into_iter()
                    .map(|(key, value)| (format!(":{}", key), value))
                    .collect(),
            ),
            condition_expression: Some(
                "attribute_exists(uaid) and (
                    attribute_not_exists(router_type) or
                    (router_type = :router_type)
                ) and (
                    attribute_not_exists(node_id) or
                    (connected_at < :connected_at)
                )"
                .to_string(),
            ),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<UserRecord>> {
        let input = GetItemInput {
            table_name: self.settings.router_table.clone(),
            consistent_read: Some(true),
            key: ddb_item! { uaid: s => uaid.simple().to_string() },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.get_item(input.clone()),
                retryable_getitem_error(self.metrics.clone()),
            )
            .await?
            .item
            .map(serde_dynamodb::from_hashmap)
            .transpose()
            .map_err(|e| {
                error!("DbClient::get_user: {:?}", e.to_string());
                DbError::from(e)
            })
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let input = DeleteItemInput {
            table_name: self.settings.router_table.clone(),
            key: ddb_item! { uaid: s => uaid.simple().to_string() },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.delete_item(input.clone()),
                retryable_delete_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let input = UpdateItemInput {
            table_name: self.settings.message_table.clone(),
            key: ddb_item! {
                uaid: s => uaid.simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            update_expression: Some("ADD chids :channel_id SET expiry = :expiry".to_string()),
            expression_attribute_values: Some(hashmap! {
                ":channel_id".to_string() => val!(SS => Some(channel_id)),
                ":expiry".to_string() => val!(N => sec_since_epoch() + MAX_CHANNEL_TTL)
            }),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn save_channels(
        &self,
        uaid: &Uuid,
        channel_list: HashSet<&Uuid>,
        _message_month: &str,
    ) -> DbResult<()> {
        let chids: Vec<String> = channel_list
            .into_iter()
            .map(|v| v.simple().to_string())
            .collect();
        let expiry = sec_since_epoch() + 2 * MAX_EXPIRY;
        let attr_values = hashmap! {
            ":chids".to_string() => val!(SS => chids),
            ":expiry".to_string() => val!(N => expiry),
        };
        let update_item = UpdateItemInput {
            key: ddb_item! {
                uaid: s => uaid.simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            update_expression: Some("ADD chids :chids SET expiry=:expiry".to_string()),
            expression_attribute_values: Some(attr_values),
            table_name: self.settings.message_table.clone(),
            ..Default::default()
        };

        self.db_client.update_item(update_item.clone()).await?;
        Ok(())
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        // Channel IDs are stored in a special row in the message table, where
        // chidmessageid = " "
        let input = GetItemInput {
            table_name: self.settings.message_table.clone(),
            consistent_read: Some(true),
            key: ddb_item! {
                uaid: s => uaid.simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            ..Default::default()
        };

        let output = retry_policy()
            .retry_if(
                || self.db_client.get_item(input.clone()),
                retryable_getitem_error(self.metrics.clone()),
            )
            .await?;

        // The channel IDs are in the notification's `chids` field
        let channels = output
            .item
            // Deserialize the notification
            .map(serde_dynamodb::from_hashmap::<NotificationRecord, _>)
            .transpose()?
            // Extract the channel IDs
            .and_then(|n| n.chids)
            .unwrap_or_default();

        // Convert the IDs from String to Uuid
        let channels = channels
            .into_iter()
            .filter_map(|s| Uuid::parse_str(&s).ok())
            .collect();

        Ok(channels)
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let input = UpdateItemInput {
            table_name: self.settings.message_table.clone(),
            key: ddb_item! {
                uaid: s => uaid.simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            update_expression: Some("DELETE chids :channel_id SET expiry = :expiry".to_string()),
            expression_attribute_values: Some(hashmap! {
                ":channel_id".to_string() => val!(SS => Some(channel_id)),
                ":expiry".to_string() => val!(N => sec_since_epoch() + MAX_CHANNEL_TTL)
            }),
            return_values: Some("UPDATED_OLD".to_string()),
            ..Default::default()
        };

        let output = retry_policy()
            .retry_if(
                || self.db_client.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;

        // Check if the old channel IDs contain the removed channel
        Ok(output
            .attributes
            .as_ref()
            .and_then(|map| map.get("chids"))
            .and_then(|item| item.ss.as_ref())
            .map(|channel_ids| channel_ids.contains(&channel_id.to_string()))
            .unwrap_or(false))
    }

    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        let input = UpdateItemInput {
            key: ddb_item! { uaid: s => uaid.simple().to_string() },
            update_expression: Some("REMOVE node_id".to_string()),
            condition_expression: Some("(node_id = :node) and (connected_at = :conn)".to_string()),
            expression_attribute_values: Some(hashmap! {
                ":node".to_string() => val!(S => node_id),
                ":conn".to_string() => val!(N => connected_at.to_string())
            }),
            table_name: self.settings.router_table.clone(),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }

    async fn fetch_messages(&self, uaid: &Uuid, limit: usize) -> DbResult<FetchMessageResponse> {
        // from commands::fetch_messages()
        let attr_values = hashmap! {
            ":uaid".to_string() => val!(S => uaid.simple().to_string()),
            ":cmi".to_string() => val!(S => "02"),
        };
        let input = QueryInput {
            key_condition_expression: Some("uaid = :uaid AND chidmessageid < :cmi".to_string()),
            expression_attribute_values: Some(attr_values),
            table_name: self.settings.message_table.to_string(),
            consistent_read: Some(true),
            limit: Some(limit as i64),
            ..Default::default()
        };

        let output = self.db_client.query(input.clone()).await?;
        let mut notifs: Vec<NotificationRecord> = output.items.map_or_else(Vec::new, |items| {
            debug!("Got response of: {:?}", items);
            items
                .into_iter()
                .inspect(|i| debug!("Item: {:?}", i))
                .filter_map(|item| {
                    let item2 = item.clone();
                    ok_or_inspect(serde_dynamodb::from_hashmap(item), |e| {
                        conversion_err(&self.metrics, e, item2, "serde_dynamodb_from_hashmap")
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
                    conversion_err(&self.metrics, e, ddb_notif2, "into_notif")
                })
            })
            .collect();
        Ok(FetchMessageResponse {
            timestamp,
            messages,
        })
    }

    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let range_key = if let Some(ts) = timestamp {
            format!("02:{}:z", ts)
        } else {
            "01;".to_string()
        };
        let attr_values = hashmap! {
            ":uaid".to_string() => val!(S => uaid.simple().to_string()),
            ":cmi".to_string() => val!(S => range_key),
        };
        let input = QueryInput {
            key_condition_expression: Some("uaid = :uaid AND chidmessageid > :cmi".to_string()),
            expression_attribute_values: Some(attr_values),
            table_name: self.settings.message_table.to_string(),
            consistent_read: Some(true),
            limit: Some(limit as i64),
            ..Default::default()
        };

        let output = self.db_client.query(input.clone()).await?;
        let messages = output.items.map_or_else(Vec::new, |items| {
            debug!("Got response of: {:?}", items);
            items
                .into_iter()
                .filter_map(|item| {
                    let item2 = item.clone();
                    ok_or_inspect(serde_dynamodb::from_hashmap(item), |e| {
                        conversion_err(&self.metrics, e, item2, "serde_dynamodb_from_hashmap")
                    })
                })
                .filter_map(|ddb_notif: NotificationRecord| {
                    let ddb_notif2 = ddb_notif.clone();
                    ok_or_inspect(ddb_notif.into_notif(), |e| {
                        conversion_err(&self.metrics, e, ddb_notif2, "into_notif")
                    })
                })
                .collect()
        });
        let timestamp = messages.iter().filter_map(|m| m.sortkey_timestamp).max();
        Ok(FetchMessageResponse {
            timestamp,
            messages,
        })
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let input = PutItemInput {
            item: serde_dynamodb::to_hashmap(&NotificationRecord::from_notif(uaid, message))?,
            table_name: self.settings.message_table.clone(),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.put_item(input.clone()),
                retryable_putitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        let input = DeleteItemInput {
            table_name: self.settings.message_table.clone(),
            key: ddb_item! {
               uaid: s => uaid.simple().to_string(),
               chidmessageid: s => sort_key.to_owned()
            },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.delete_item(input.clone()),
                retryable_delete_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    /// Perform the "hello" registration process.
    /// Each storage engine can be different, so the 'hello' function needs to be
    /// specific to the engine, unfortunately.
    ///
    async fn hello(
        &self,
        connected_at: u64,
        uaid: Option<&Uuid>,
        router_url: &str,
        mut defer_registration: bool,
    ) -> DbResult<HelloResponse> {
        let cur_month = self.settings.current_message_month.clone();
        // lookup_user
        let mut response = HelloResponse {
            message_month: cur_month.clone(),
            connected_at,
            ..Default::default()
        };
        if let Some(uaid) = uaid {
            // Get the user record
            let user = self.get_user(uaid).await;
            // command: handle_user_result
            //  if no user (false, 104)
            //  if unparsable user (true, 104)
            //  no user.current_month (true, 104)
            //  user.current_month not in message_tables (true, 105)
            //
            // true = `ua.expiration` && drop_user(uaid)
            match user {
                Ok(None) => {
                    // No user found, so return the base response we have.
                    // (false, 104)
                    // bail out and return the stub response we've generated.
                }
                Ok(Some(mut user)) => {
                    // We have a user record. Update it to include the latest info.
                    if self
                        .settings
                        .message_table_names
                        .contains(&self.settings.current_message_month)
                    {
                        if let Some(user_month) = user.current_month.clone() {
                            response.uaid = Some(user.uaid);
                            // the user's month is current, hopefully you don't have to migrate.
                            if self.settings.message_table_names.contains(&user_month) {
                                response.check_storage = true;
                                response.rotate_message_table = user_month != cur_month;
                                response.message_month = user_month;
                                response.reset_uaid = user
                                    .record_version
                                    .map_or(true, |rec_ver| rec_ver < USER_RECORD_VERSION);
                                // update the current user record.
                                user.last_connect = if has_connected_this_month(&user) {
                                    None
                                } else {
                                    Some(generate_last_connect())
                                };
                                user.node_id = Some(router_url.to_owned());
                                user.connected_at = connected_at;
                                self.update_user(&user).await?;
                                // The user is already registered. Make sure not to re-add by
                                // force clearing the defer_registration flag
                                defer_registration = false;
                            } else {
                                // The user's current month has aged out of our list of supported months.
                                // (true, 105)
                                trace!("🧑 handle_user_result {}: {:?}", &uaid, "105");
                                self.metrics
                                    .incr_with_tags("ua.expiration")
                                    .with_tag("code", "105")
                                    .send();
                                self.remove_user(uaid).await?;
                            }
                        } else {
                            // user.current_month is None
                            // (true, 105)
                            trace!("🧑 handle_user_result {}: {:?}", &uaid, "105");
                            self.metrics
                                .incr_with_tags("ua.expiration")
                                .with_tag("code", "105")
                                .send();
                            self.remove_user(uaid).await?;
                        }
                    }
                    if !defer_registration {
                        self.add_user(&user).await?;
                    } else {
                        response.deferred_user_registration = Some(user);
                    }
                }
                Err(e) => {
                    self.metrics
                        .incr_with_tags("ua.expiration")
                        .with_tag("code", "104")
                        .send();
                    self.remove_user(uaid).await?;
                    return Err(e);
                }
            }
        }
        Ok(response)
    }

    async fn router_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.settings.router_table.clone()).await
    }

    async fn message_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.settings.message_table.clone()).await
    }

    fn message_table(&self) -> &str {
        trace!("ddb message table {:?}", &self.settings.message_table);
        &self.settings.message_table
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }
}

/// Indicate whether this last_connect falls in the current month
fn has_connected_this_month(user: &UserRecord) -> bool {
    user.last_connect.map_or(false, |v| {
        let pat = Utc::now().format("%Y%m").to_string();
        v.to_string().starts_with(&pat)
    })
}
