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

/// Create a new mock DB client. Arc is used so that the mock can be cloned.
pub fn mock_db_client() -> Arc<MockDbClient> {
    Arc::new(MockDbClient::new())
}

// mockall currently has issues mocking async traits with #[automock], so we use
// this workaround. See https://github.com/asomers/mockall/issues/75
mockall::mock! {
    pub DbClient {
        fn get_user(&self, uaid: Uuid) -> DbResult<Option<DynamoDbUser>>;

        fn remove_user(&self, uaid: Uuid) -> DbResult<()>;

        fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>>;

        fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()>;

        fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()>;

        fn message_table(&self) -> &str;

        fn box_clone(&self) -> Box<dyn DbClient>;
    }
}

#[async_trait]
impl DbClient for Arc<MockDbClient> {
    async fn get_user(&self, uaid: Uuid) -> DbResult<Option<DynamoDbUser>> {
        Arc::as_ref(self).get_user(uaid)
    }

    async fn remove_user(&self, uaid: Uuid) -> DbResult<()> {
        Arc::as_ref(self).remove_user(uaid)
    }

    async fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>> {
        Arc::as_ref(self).get_channels(uaid)
    }

    async fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()> {
        Arc::as_ref(self).remove_node_id(uaid, node_id, connected_at)
    }

    async fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()> {
        Arc::as_ref(self).save_message(uaid, message)
    }

    fn message_table(&self) -> &str {
        Arc::as_ref(self).message_table()
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(Arc::clone(self))
    }
}
