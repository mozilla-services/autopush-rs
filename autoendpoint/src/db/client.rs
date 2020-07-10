use crate::db::error::{DbError, DbResult};
use crate::db::retry::{
    retry_policy, retryable_delete_error, retryable_getitem_error, retryable_putitem_error,
    retryable_updateitem_error,
};
use autopush_common::db::{DynamoDbNotification, DynamoDbUser};
use autopush_common::notification::Notification;
use autopush_common::{ddb_item, hashmap, val};
use cadence::StatsdClient;
use rusoto_core::credential::StaticProvider;
use rusoto_core::{HttpClient, Region};
use rusoto_dynamodb::{
    AttributeValue, DeleteItemInput, DynamoDb, DynamoDbClient, GetItemInput, PutItemInput,
    UpdateItemInput,
};
use std::collections::HashSet;
use std::env;
use uuid::Uuid;

/// Provides high-level operations over the DynamoDB database
#[derive(Clone)]
pub struct DbClient {
    ddb: DynamoDbClient,
    metrics: StatsdClient,
    router_table: String,
    pub message_table: String,
}

impl DbClient {
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

    /// Read a user from the database
    pub async fn get_user(&self, uaid: Uuid) -> DbResult<Option<DynamoDbUser>> {
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

    /// Delete a user from the router table
    pub async fn drop_user(&self, uaid: Uuid) -> DbResult<()> {
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

    /// Get the set of channel IDs for a user
    pub async fn get_user_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>> {
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
            .map(serde_dynamodb::from_hashmap::<DynamoDbNotification, _>)
            .transpose()?
            // Extract the channel IDs
            .and_then(|n| n.chids)
            .unwrap_or_default();

        // Convert the IDs from String to Uuid
        let channels = channels
            .into_iter()
            .map(|s| Uuid::parse_str(&s))
            .filter_map(Result::ok)
            .collect();

        Ok(channels)
    }

    /// Remove the node ID from a user in the router table.
    /// The node ID will only be cleared if `connected_at` matches up with the
    /// item's `connected_at`.
    pub async fn remove_node_id(
        &self,
        uaid: Uuid,
        node_id: String,
        connected_at: u64,
    ) -> DbResult<()> {
        let update_item = UpdateItemInput {
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
                || self.ddb.update_item(update_item.clone()),
                retryable_updateitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }

    /// Store a single message
    pub async fn store_message(&self, uaid: Uuid, message: Notification) -> DbResult<()> {
        let put_item = PutItemInput {
            item: serde_dynamodb::to_hashmap(&DynamoDbNotification::from_notif(&uaid, message))
                .unwrap(),
            table_name: self.message_table.clone(),
            ..Default::default()
        };

        retry_policy()
            .retry_if(
                || self.ddb.put_item(put_item.clone()),
                retryable_putitem_error(self.metrics.clone()),
            )
            .await?;

        Ok(())
    }
}
