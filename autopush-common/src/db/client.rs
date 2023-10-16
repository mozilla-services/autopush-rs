use std::collections::HashSet;
use std::fmt::Debug;

use async_trait::async_trait;
use mockall::automock;
use uuid::Uuid;

use crate::db::error::DbResult;
use crate::db::User;
use crate::notification::Notification;

#[derive(Default, Debug)]
pub struct FetchMessageResponse {
    pub timestamp: Option<u64>,
    pub messages: Vec<Notification>,
}

/// Provides high-level operations for data management.
///
/// This is usually manifested by _database_::DbClientImpl
///
#[automock] // must appear before #[async_trait]
#[async_trait]
pub trait DbClient: Send + Sync {
    /// Add a new user to the database. An error will occur if the user already
    /// exists.
    async fn add_user(&self, user: &User) -> DbResult<()>;

    /// Update a user in the database. An error will occur if the user does not
    /// already exist, has a different router type, or has a newer
    /// `connected_at` timestamp.
    async fn update_user(&self, user: &User) -> DbResult<()>;

    /// Read a user from the database
    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>>;

    /// Delete a user from the router table
    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()>;

    /// Add a channel to a user
    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()>;

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

    /// Save multiple messages to the message table
    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()>;

    /// Fetch stored messages for a user
    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse>;

    /// Fetch stored messages later than a given
    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse>;

    /// Update the last read timestamp for a user
    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()>;

    /// Delete a notification
    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()>;

    /// Check if the router table exists
    async fn router_table_exists(&self) -> DbResult<bool>;

    /// Check if the message table exists
    async fn message_table_exists(&self) -> DbResult<bool>;

    /// Perform the health check on this data store
    async fn health_check(&self) -> DbResult<bool>;

    /// Return the DynamoDB current message table name
    ///
    /// DynamoDB tables were previously rotated to new tables on a monthly
    /// basis. The current table name is used to validate the DynamoDB specific
    /// legacy `User::current_month` value.
    ///
    /// N/A to BigTable (returns `None`).
    // #[automock] requires an explicit 'a lifetime here which is otherwise
    // unnecessary and rejected by clippy
    #[allow(clippy::needless_lifetimes)]
    fn rotating_message_table<'a>(&'a self) -> Option<&'a str>;

    fn box_clone(&self) -> Box<dyn DbClient>;

    /// Provide the module name.
    /// This was added for simple dual mode testing, but may be useful in
    /// other situations.
    fn name(&self) -> String;
}

impl Clone for Box<dyn DbClient> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}
