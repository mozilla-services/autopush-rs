// mockall::mock currently generates these warnings
#![allow(clippy::unused_unit)]
#![allow(clippy::ptr_arg)]

use crate::db::client::DbClient;
pub use crate::db::client::MockDbClient;
use crate::db::error::DbResult;
use crate::db::User;
use crate::notification::Notification;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use super::client::FetchMessageResponse;

#[async_trait]
impl DbClient for Arc<MockDbClient> {
    async fn add_user(&self, user: &User) -> DbResult<()> {
        Arc::as_ref(self).add_user(user).await
    }

    async fn update_user(&self, user: &User) -> DbResult<()> {
        Arc::as_ref(self).update_user(user).await
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        Arc::as_ref(self).get_user(uaid).await
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        Arc::as_ref(self).remove_user(uaid).await
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        Arc::as_ref(self).add_channel(uaid, channel_id).await
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        Arc::as_ref(self).get_channels(uaid).await
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        Arc::as_ref(self).remove_channel(uaid, channel_id).await
    }

    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        Arc::as_ref(self)
            .remove_node_id(uaid, node_id, connected_at)
            .await
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        Arc::as_ref(self).save_message(uaid, message).await
    }

    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        Arc::as_ref(self).save_messages(uaid, messages).await
    }

    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        Arc::as_ref(self).fetch_topic_messages(uaid, limit).await
    }

    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        Arc::as_ref(self)
            .fetch_timestamp_messages(uaid, timestamp, limit)
            .await
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        Arc::as_ref(self).increment_storage(uaid, timestamp).await
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        Arc::as_ref(self).remove_message(uaid, sort_key).await
    }

    async fn router_table_exists(&self) -> DbResult<bool> {
        Arc::as_ref(self).router_table_exists().await
    }

    async fn message_table_exists(&self) -> DbResult<bool> {
        Arc::as_ref(self).message_table_exists().await
    }

    fn rotating_message_table(&self) -> Option<&str> {
        Arc::as_ref(self).rotating_message_table()
    }

    async fn health_check(&self) -> DbResult<bool> {
        Arc::as_ref(self).health_check().await
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(Arc::clone(self))
    }
}

impl MockDbClient {
    /// Convert into a type which can be used in place of `Box<dyn DbClient>`.
    /// Arc is used so that the mock can be cloned. Box is used so it can be
    /// easily cast to `Box<dyn DbClient>`.
    #[allow(clippy::redundant_allocation)]
    pub fn into_boxed_arc(self) -> Box<Arc<Self>> {
        Box::new(Arc::new(self))
    }
}
