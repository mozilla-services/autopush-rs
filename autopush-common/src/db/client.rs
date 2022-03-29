use crate::db::error::DbResult;
use crate::db::UserRecord;
use crate::notification::Notification;
use async_trait::async_trait;
use std::collections::HashSet;
use uuid::Uuid;

/// Provides high-level operations over the DynamoDB database
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
    async fn get_user(&self, uaid: Uuid) -> DbResult<Option<UserRecord>>;

    /// Delete a user from the router table
    async fn remove_user(&self, uaid: Uuid) -> DbResult<()>;

    /// Add a channel to a user
    async fn add_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<()>;

    /// Get the set of channel IDs for a user
    async fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>>;

    /// Remove a channel from a user. Returns if the removed channel did exist.
    async fn remove_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<bool>;

    /// Remove the node ID from a user in the router table.
    /// The node ID will only be cleared if `connected_at` matches up with the
    /// item's `connected_at`.
    async fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()>;

    /// Save a message to the message table
    async fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()>;

    /// Delete a notification
    async fn remove_message(&self, uaid: Uuid, sort_key: String) -> DbResult<()>;

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
