use crate::db::error::DbResult;
use crate::db::UserRecord;
use crate::notification::Notification;
use async_trait::async_trait;
use std::collections::HashSet;
use uuid::Uuid;

use super::HelloResponse;

#[derive(Default, Debug)]
pub struct FetchMessageResponse {
    pub timestamp: Option<u64>,
    pub messages: Vec<Notification>,
}

/// Provides high-level operations for data management.
///
/// This is usually manifested by _database_::DbClientImpl
///
#[async_trait]
pub trait DbClient: Send + Sync {
    /// Add a new user to the database. An error will occur if the user already
    /// exists.
    async fn add_user(&self, user: &UserRecord) -> DbResult<()>;

    /// Update a user in the database. An error will occur if the user does not
    /// already exist, has a different router type, or has a newer
    /// `connected_at` timestamp.
    async fn update_user(&self, user: &UserRecord) -> DbResult<()>;

    /// Read a user from the database
    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<UserRecord>>;

    /// Delete a user from the router table
    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()>;

    /// Add a channel to a user
    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()>;

    /// Replace the current channel list
    async fn save_channels(
        &self,
        uaid: &Uuid,
        channel_list: HashSet<&Uuid>,
        message_month: &str,
    ) -> DbResult<()>;

    /// Get the set of channel IDs for a user
    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>>;

    /// Remove a channel from a user. Returns if the removed channel did exist.
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool>;

    /// Remove the node ID from a user in the router table.
    /// The node ID will only be cleared if `connected_at` matches up with the
    /// item's `connected_at`.
    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()>;

    /// Save a message to the message table
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()>;

    /// Fetch stored messages for a user
    async fn fetch_messages(&self, uaid: &Uuid, limit: usize) -> DbResult<FetchMessageResponse>;

    /// Fetch stored messages later than a given
    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse>;

    /// Delete a notification
    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()>;

    /// record a Hello record
    /// Each data store can handle this differently, thus it's best to hand things off to the engine.
    async fn hello(&self, connected_at: u64, uaid: Option<&Uuid>, router_url: &str, defer_registration: bool) -> DbResult<HelloResponse>;

    /// Check if the router table exists
    async fn router_table_exists(&self) -> DbResult<bool>;

    /// Check if the message table exists
    async fn message_table_exists(&self) -> DbResult<bool>;

    /// Get the message table name
    fn message_table(&self) -> &str;

    fn box_clone(&self) -> Box<dyn DbClient>;
}

impl Clone for Box<dyn DbClient> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}
