use crate::db::client::{DbClient, MAX_CHANNEL_TTL};
use crate::db::error::{DbError, DbResult};
use crate::db::retry::{
    retry_policy, retryable_delete_error, retryable_describe_table_error, retryable_getitem_error,
    retryable_putitem_error, retryable_updateitem_error,
};
use async_trait::async_trait;
use autopush_common::db::{NotificationRecord, UserRecord};
use autopush_common::notification::Notification;
use autopush_common::util::sec_since_epoch;
use autopush_common::{ddb_item, hashmap, val};
use cadence::StatsdClient;
use rusoto_core::credential::StaticProvider;
use rusoto_core::{HttpClient, Region, RusotoError};
use rusoto_dynamodb::{
    AttributeValue, DeleteItemInput, DescribeTableError, DescribeTableInput, DynamoDb,
    DynamoDbClient, GetItemInput, PutItemInput, UpdateItemInput,
};
use std::collections::HashSet;
use std::env;
use uuid::Uuid;


#[derive(Clone)]
pub struct DdbClientImpl {
    ddb: DynamoDbClient,
    metrics: StatsdClient,
    router_table: String,
    message_table: String,
}

impl DdbClientImpl {
    pub fn new(
        metrics: StatsdClient,
        router_table: String,
        message_table: String,
    ) -> DbResult<Self> {
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

        Ok(Self {
            ddb,
            metrics,
            router_table,
            message_table,
        })
    }

    /// Check if a table exists
    async fn table_exists(&self, table_name: String) -> DbResult<bool> {
        let input = DescribeTableInput { table_name };

        let output = match retry_policy()
            .retry_if(
                || self.ddb.describe_table(input.clone()),
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

#[allow(clippy::field_reassign_with_default)]
#[async_trait]
impl DbClient for DdbClientImpl {
    async fn add_user(&self, user: &UserRecord) -> DbResult<()> {
        let input = PutItemInput {
            table_name: self.router_table.clone(),
            item: serde_dynamodb::to_hashmap(user)?,
            condition_expression: Some("attribute_not_exists(uaid)".to_string()),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.put_item(input.clone()),
                retryable_putitem_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn update_user(&self, user: &UserRecord) -> DbResult<()> {
        let mut user_map = serde_dynamodb::to_hashmap(&user)?;
        user_map.remove("uaid");
        let input = UpdateItemInput {
            table_name: self.router_table.clone(),
            key: ddb_item! { uaid: s => user.uaid.to_simple().to_string() },
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
                || self.ddb.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn get_user(&self, uaid: Uuid) -> DbResult<Option<UserRecord>> {
        let input = GetItemInput {
            table_name: self.router_table.clone(),
            consistent_read: Some(true),
            key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.get_item(input.clone()),
                retryable_getitem_error(self.metrics.clone()),
            )
            .await?
            .item
            .map(serde_dynamodb::from_hashmap)
            .transpose()
            .map_err(DbError::from)
    }

    async fn remove_user(&self, uaid: Uuid) -> DbResult<()> {
        let input = DeleteItemInput {
            table_name: self.router_table.clone(),
            key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.delete_item(input.clone()),
                retryable_delete_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn add_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<()> {
        let input = UpdateItemInput {
            table_name: self.message_table.clone(),
            key: ddb_item! {
                uaid: s => uaid.to_simple().to_string(),
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
                || self.ddb.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>> {
        // Channel IDs are stored in a special row in the message table, where
        // chidmessageid = " "
        let input = GetItemInput {
            table_name: self.message_table.clone(),
            consistent_read: Some(true),
            key: ddb_item! {
                uaid: s => uaid.to_simple().to_string(),
                chidmessageid: s => " ".to_string()
            },
            ..Default::default()
        };

        let output = retry_policy()
            .retry_if(
                || self.ddb.get_item(input.clone()),
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

    async fn remove_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<bool> {
        let input = UpdateItemInput {
            table_name: self.message_table.clone(),
            key: ddb_item! {
                uaid: s => uaid.to_simple().to_string(),
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
                || self.ddb.update_item(input.clone()),
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

    async fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()> {
        let input = UpdateItemInput {
            key: ddb_item! { uaid: s => uaid.to_simple().to_string() },
            update_expression: Some("REMOVE node_id".to_string()),
            condition_expression: Some("(node_id = :node) and (connected_at = :conn)".to_string()),
            expression_attribute_values: Some(hashmap! {
                ":node".to_string() => val!(S => node_id),
                ":conn".to_string() => val!(N => connected_at.to_string())
            }),
            table_name: self.router_table.clone(),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.update_item(input.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }

    async fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()> {
        let input = PutItemInput {
            item: serde_dynamodb::to_hashmap(&NotificationRecord::from_notif(&uaid, message))?,
            table_name: self.message_table.clone(),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.put_item(input.clone()),
                retryable_putitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }

    async fn remove_message(&self, uaid: Uuid, sort_key: String) -> DbResult<()> {
        let input = DeleteItemInput {
            table_name: self.message_table.clone(),
            key: ddb_item! {
               uaid: s => uaid.to_simple().to_string(),
               chidmessageid: s => sort_key
            },
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.delete_item(input.clone()),
                retryable_delete_error(self.metrics.clone()),
            )
            .await?;
        Ok(())
    }

    async fn router_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.router_table.clone()).await
    }

    async fn message_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.message_table.clone()).await
    }

    fn message_table(&self) -> &str {
        &self.message_table
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }
}
