// mockall::mock currently generates these warnings
#![allow(clippy::unused_unit)]
#![allow(clippy::ptr_arg)]

use crate::db::client::DbClient;
use crate::db::error::DbResult;
use crate::db::User;
use crate::notification::Notification;
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use super::client::FetchMessageResponse;
use super::HelloResponse;

// mockall currently has issues mocking async traits with #[automock], so we use
// this workaround. See https://github.com/asomers/mockall/issues/75
mockall::mock! {
    pub DbClient {
        fn current_message_month(&self) -> Option<String>;

        fn message_tables(&self) -> &Vec<String>;

        fn add_user(&self, user: &User) -> DbResult<()>;

        fn update_user(&self, user: &User) -> DbResult<()>;

        fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>>;

        fn remove_user(&self, uaid: &Uuid) -> DbResult<()>;

        fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()>;

        fn save_channels(&self, uaid: &Uuid, channel_list: HashSet<Uuid>, message_month: &str) -> DbResult<()>;

        fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>>;

        fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool>;

        fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()>;

        fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()>;

        fn fetch_messages(&self, uaid: &Uuid, limit: usize) -> DbResult<FetchMessageResponse>;

        fn fetch_timestamp_messages(
            &self,
            uaid: &Uuid,
            timestamp: Option<u64>,
            limit: usize,
        ) -> DbResult<FetchMessageResponse>;

        fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()>;

        fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()>;

        fn hello(&self, connected_at: u64, uaid: Option<Uuid>, router_url: &str, defer_registration: bool) -> DbResult<HelloResponse>;

        fn router_table_exists(&self) -> DbResult<bool>;

        fn message_table_exists(&self) -> DbResult<bool>;

        fn message_table(&self) -> &str;

        fn box_clone(&self) -> Box<dyn DbClient>;
    }
}

#[async_trait]
impl DbClient for Arc<MockDbClient> {
    fn current_message_month(&self) -> Option<String> {
        Arc::as_ref(self).current_message_month()
    }

    fn message_tables(&self) -> &Vec<String> {
        Arc::as_ref(self).message_tables()
    }

    async fn add_user(&self, user: &User) -> DbResult<()> {
        Arc::as_ref(self).add_user(user)
    }

    async fn update_user(&self, user: &User) -> DbResult<()> {
        Arc::as_ref(self).update_user(user)
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        Arc::as_ref(self).get_user(uaid)
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        Arc::as_ref(self).remove_user(uaid)
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        Arc::as_ref(self).add_channel(uaid, channel_id)
    }

    async fn save_channels(
        &self,
        uaid: &Uuid,
        channel_list: HashSet<&Uuid>,
        message_month: &str,
    ) -> DbResult<()> {
        // because `HashSet<&Uuid>` is not an iterator, so `.map()` doesn't work.`
        let mut clist: HashSet<Uuid> = HashSet::new();
        for v in channel_list {
            clist.insert(*v);
        }
        Arc::as_ref(self).save_channels(uaid, clist, message_month)
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        Arc::as_ref(self).get_channels(uaid)
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        Arc::as_ref(self).remove_channel(uaid, channel_id)
    }

    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        Arc::as_ref(self).remove_node_id(uaid, node_id, connected_at)
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        Arc::as_ref(self).save_message(uaid, message)
    }

    async fn fetch_messages(&self, uaid: &Uuid, limit: usize) -> DbResult<FetchMessageResponse> {
        Arc::as_ref(self).fetch_messages(uaid, limit)
    }

    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        Arc::as_ref(self).fetch_timestamp_messages(uaid, timestamp, limit)
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        Arc::as_ref(self).increment_storage(uaid, timestamp)
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        Arc::as_ref(self).remove_message(uaid, sort_key)
    }

    async fn hello(
        &self,
        connected_at: u64,
        uaid: Option<&Uuid>,
        router_url: &str,
        defer_registration: bool,
    ) -> DbResult<HelloResponse> {
        Arc::as_ref(self).hello(connected_at, uaid.copied(), router_url, defer_registration)
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
