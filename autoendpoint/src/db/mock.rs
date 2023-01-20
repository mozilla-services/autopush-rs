// mockall::mock currently generates these warnings
#![allow(clippy::unused_unit)]
#![allow(clippy::ptr_arg)]

use crate::db::client::DbClient;
use crate::db::error::DbResult;
use async_trait::async_trait;
use autopush_common::db::DynamoDbUser;
use autopush_common::notification::Notification;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

// mockall currently has issues mocking async traits with #[automock], so we use
// this workaround. See https://github.com/asomers/mockall/issues/75
mockall::mock! {
    pub DbClient {
        fn add_user(&self, user: &DynamoDbUser) -> DbResult<()>;

        fn update_user(&self, user: &mut DynamoDbUser) -> DbResult<()>;

        fn get_user(&self, uaid: Uuid) -> DbResult<Option<DynamoDbUser>>;

        fn remove_user(&self, uaid: Uuid) -> DbResult<()>;

        fn add_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<()>;

        fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>>;

        fn remove_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<bool>;

        fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()>;

        fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()>;

        fn remove_message(&self, uaid: Uuid, sort_key: String) -> DbResult<()>;

        fn router_table_exists(&self) -> DbResult<bool>;

        fn message_table_exists(&self) -> DbResult<bool>;

        fn message_table(&self) -> &str;

        fn box_clone(&self) -> Box<dyn DbClient>;
    }
}

#[async_trait]
impl DbClient for Arc<MockDbClient> {
    async fn add_user(&self, user: &DynamoDbUser) -> DbResult<()> {
        Arc::as_ref(self).add_user(user)
    }

    async fn update_user(&self, user: &mut DynamoDbUser) -> DbResult<()> {
        Arc::as_ref(self).update_user(user)
    }

    async fn get_user(&self, uaid: Uuid) -> DbResult<Option<DynamoDbUser>> {
        Arc::as_ref(self).get_user(uaid)
    }

    async fn remove_user(&self, uaid: Uuid) -> DbResult<()> {
        Arc::as_ref(self).remove_user(uaid)
    }

    async fn add_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<()> {
        Arc::as_ref(self).add_channel(uaid, channel_id)
    }

    async fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>> {
        Arc::as_ref(self).get_channels(uaid)
    }

    async fn remove_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<bool> {
        Arc::as_ref(self).remove_channel(uaid, channel_id)
    }

    async fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()> {
        Arc::as_ref(self).remove_node_id(uaid, node_id, connected_at)
    }

    async fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()> {
        Arc::as_ref(self).save_message(uaid, message)
    }

    async fn remove_message(&self, uaid: Uuid, sort_key: String) -> DbResult<()> {
        Arc::as_ref(self).remove_message(uaid, sort_key)
    }

    async fn router_table_exists(&self) -> DbResult<bool> {
        Arc::as_ref(self).router_table_exists()
    }

    async fn message_table_exists(&self) -> DbResult<bool> {
        Arc::as_ref(self).message_table_exists()
    }

    fn message_table(&self) -> &str {
        Arc::as_ref(self).message_table()
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
