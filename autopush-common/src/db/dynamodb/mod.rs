use std::collections::HashSet;
use std::env;
use std::fmt::{Debug, Display};
use std::result::Result as StdResult;
use std::sync::Arc;

use crate::db::client::DbClient;
use crate::db::dynamodb::retry::{
    retry_policy, retryable_batchwriteitem_error, retryable_delete_error,
    retryable_describe_table_error, retryable_getitem_error, retryable_putitem_error,
    retryable_query_error, retryable_updateitem_error,
};
use crate::db::error::{DbError, DbResult};
use crate::db::{
    client::FetchMessageResponse, DbSettings, NotificationRecord, User, MAX_CHANNEL_TTL, MAX_EXPIRY,
};
use crate::notification::Notification;
use crate::util::sec_since_epoch;

use async_trait::async_trait;
use cadence::{CountedExt, StatsdClient};
use chrono::Utc;
use rusoto_core::credential::StaticProvider;
use rusoto_core::{HttpClient, Region, RusotoError};
use rusoto_dynamodb::{
    AttributeValue, BatchWriteItemInput, DeleteItemInput, DescribeTableError, DescribeTableInput,
    DynamoDb, DynamoDbClient, GetItemInput, ListTablesInput, PutItemInput, PutRequest, QueryInput,
    UpdateItemError, UpdateItemInput, WriteRequest,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[macro_use]
pub mod macros;
pub mod retry;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DynamoDbSettings {
    #[serde(default)]
    pub router_table: String,
    #[serde(default)]
    pub message_table: String,
    #[serde(default)]
    pub db_routing_table: Option<String>,
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
    pub fn new(metrics: Arc<StatsdClient>, db_settings: &DbSettings) -> DbResult<Self> {
        debug!("🛢️DynamoDB Settings {:?}", db_settings);
        let db_client = if let Ok(endpoint) = env::var("AWS_LOCAL_DYNAMODB") {
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

        let settings = DynamoDbSettings::try_from(db_settings.db_settings.as_ref())?;
        Ok(Self {
            db_client,
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
    async fn add_user(&self, user: &User) -> DbResult<()> {
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

    async fn update_user(&self, user: &mut User) -> DbResult<bool> {
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

        let result = retry_policy()
            .retry_if(
                || self.db_client.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(RusotoError::Service(UpdateItemError::ConditionalCheckFailed(_))) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
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
            .await
            .map_err(|e| {
                error!(
                    "add_channel failed; uaid: {:?}, channel_id: {:?}",
                    uaid, channel_id
                );
                e
            })?;
        Ok(())
    }

    /// Hopefully, this is never called. It is provided for completion sake.
    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        for channel_id in channels {
            self.add_channel(uaid, &channel_id).await?;
        }
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
            .await
            .map_err(|e| {
                error!(
                    "remove_channel failed: uaid:{:?}, chid:{:?}",
                    uaid, channel_id
                );
                e
            })?;

        // Check if the old channel IDs contain the removed channel
        Ok(output
            .attributes
            .as_ref()
            .and_then(|map| map.get("chids"))
            .and_then(|item| item.ss.as_ref())
            .map(|channel_ids| channel_ids.contains(&channel_id.to_string()))
            .unwrap_or(false))
    }

    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        node_id: &str,
        connected_at: u64,
        _version: &Option<Uuid>,
    ) -> DbResult<bool> {
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

        let result = retry_policy()
            .retry_if(
                || self.db_client.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await;
        match result {
            Ok(_) => Ok(true),
            Err(RusotoError::Service(UpdateItemError::ConditionalCheckFailed(_))) => Ok(false),
            Err(e) => {
                error!(
                    "remove_node_id failed. node_id:{:?}, connected_at:{:?}",
                    node_id, connected_at
                );
                Err(e.into())
            }
        }
    }

    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        // from commands::fetch_topic_messages()
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

        let output = retry_policy()
            .retry_if(
                || self.db_client.query(input.clone()),
                retryable_query_error(self.metrics.clone()),
            )
            .await?;

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
        // Specify the minimum value to look for as a channelmessageid
        let range_key = if let Some(ts) = timestamp {
            format!("02:{}:z", ts)
        } else {
            // fun ascii tricks? because ':' comes before ';', look for any non-topic?
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

        let output = retry_policy()
            .retry_if(
                || self.db_client.query(input.clone()),
                retryable_query_error(self.metrics.clone()),
            )
            .await?;

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

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        let expiry = sec_since_epoch() + 2 * MAX_EXPIRY;
        let attr_values = hashmap! {
            ":timestamp".to_string() => val!(N => timestamp.to_string()),
            ":expiry".to_string() => val!(N => expiry),
        };
        let update_input = UpdateItemInput {
            key: ddb_item! {
                uaid: s => uaid.as_simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            update_expression: Some(
                "SET current_timestamp = :timestamp, expiry = :expiry".to_string(),
            ),
            expression_attribute_values: Some(attr_values),
            table_name: self.settings.message_table.clone(),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.update_item(update_input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let topic = message.topic.is_some().to_string();
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

        self.metrics
            .incr_with_tags("notification.message.stored")
            .with_tag("topic", &topic)
            .with_tag("database", &self.name())
            .send();
        Ok(())
    }

    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        let put_items: Vec<WriteRequest> = messages
            .into_iter()
            .filter_map(|n| {
                // eventually include `internal` if `meta` defined.
                self.metrics
                    .incr_with_tags("notification.message.stored")
                    .with_tag("topic", &n.topic.is_some().to_string())
                    .with_tag("database", &self.name())
                    .send();
                serde_dynamodb::to_hashmap(&NotificationRecord::from_notif(uaid, n))
                    .ok()
                    .map(|hm| WriteRequest {
                        put_request: Some(PutRequest { item: hm }),
                        delete_request: None,
                    })
            })
            .collect();
        let batch_input = BatchWriteItemInput {
            request_items: hashmap! { self.settings.message_table.clone() => put_items },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.db_client.batch_write_item(batch_input.clone()),
                retryable_batchwriteitem_error(self.metrics.clone()),
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
        self.metrics
            .incr_with_tags("notification.message.deleted")
            .with_tag("database", &self.name())
            .send();
        Ok(())
    }

    async fn router_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.settings.router_table.clone()).await
    }

    async fn message_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.settings.message_table.clone()).await
    }

    fn rotating_message_table(&self) -> Option<&str> {
        trace!("ddb message table {:?}", &self.settings.message_table);
        Some(&self.settings.message_table)
    }

    /// Perform a simple health check to make sure that the database is there.
    /// This is called by __health__, so it should be reasonably light weight.
    async fn health_check(&self) -> DbResult<bool> {
        let input = ListTablesInput {
            exclusive_start_table_name: Some(self.settings.message_table.clone()),
            ..Default::default()
        };
        // if we can't connect, that's a fail.
        let result = self
            .db_client
            .list_tables(input)
            .await
            .map_err(|e| DbError::General(format!("DynamoDB health check failure: {:?}", e)))?;
        if let Some(names) = result.table_names {
            // We found at least one table that matches the message_table
            debug!("dynamodb ok");
            return Ok(!names.is_empty());
        }
        // Huh, we couldn't find a message table? That's a failure.
        return Err(DbError::General(
            "DynamoDB health check failure: No message table found".to_owned(),
        ));
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }

    fn name(&self) -> String {
        "DynamoDb".to_owned()
    }
}

/// Indicate whether this last_connect falls in the current month
pub(crate) fn has_connected_this_month(user: &User) -> bool {
    user.last_connect.map_or(false, |v| {
        let pat = Utc::now().format("%Y%m").to_string();
        v.to_string().starts_with(&pat)
    })
}
