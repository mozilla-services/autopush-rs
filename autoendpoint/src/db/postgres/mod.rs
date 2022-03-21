use std::collections::HashSet;
use std::env;
use std::panic::panic_any;
use std::sync::Arc;

use tokio_postgres::Client; // Client is sync.

use async_trait::async_trait;
use cadence::StatsdClient;
use uuid::Uuid;

use crate::db::client::DbClient;
use crate::db::error::{DbError, DbResult};
use crate::db::retry::{
    retry_policy, retryable_delete_error, retryable_describe_table_error, retryable_getitem_error,
    retryable_putitem_error, retryable_updateitem_error,
};
use crate::settings::Settings;
use autopush_common::db::{NotificationRecord, UserRecord, MAX_CHANNEL_TTL};
use autopush_common::notification::Notification;
use autopush_common::util::sec_since_epoch;
use autopush_common::{ddb_item, hashmap, val};

#[derive(Clone)]
pub struct PgClientImpl {
    client: Arc<Client>,
    metrics: Arc<StatsdClient>,
    router_table: String,  // Routing information
    message_table: String, // Message storage information
    meta_table: String,    // Channels and meta info for a user.
}

impl PgClientImpl {
    pub async fn new(metrics: Arc<StatsdClient>, settings: Settings) -> DbResult<Self> {
        if let Some(dsn) = settings.pg_dsn {
            let parsed = url::Url::parse(&dsn)
                .map_err(|e| DbError::Connection(format!("Invalid Postgres DSN: {:?}", e)))?;
            let pg_connect = format!(
                "user={:?} password={:?} host={:?} port={:?} dbname={:?}",
                parsed.username(),
                parsed.password().unwrap_or_default(),
                parsed.host_str().unwrap_or_default(),
                parsed.port(),
                parsed
                    .path_segments()
                    .map(|c| c.collect::<Vec<_>>())
                    .unwrap_or_default()[0]
            );
            let (client, connection) = tokio_postgres::connect(&pg_connect, tokio_postgres::NoTls)
                .await
                .map_err(|e| {
                    DbError::Connection(format!("Could not connect to postgres {:?}", e))
                })?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    panic_any(format!("PG Connection error {:?}", e));
                }
            });
            return Ok(Self {
                client: Arc::new(client),
                metrics,
                router_table: settings.router_table_name,
                message_table: settings.message_table_name,
                meta_table: settings.meta_table_name.unwrap_or("meta".to_owned()),
            });
        };
        Err(DbError::Connection("No DSN specified".to_owned()))
    }

    async fn table_exists(&self, table_name: String) -> DbResult<bool> {
        let rows = self
            .client
            .query(
                "SELECT EXISTS (SELECT FROM pg_tables where schemaname='public' AND tablename=$1);",
                &[&table_name],
            )
            .await?;
        let val: &str = rows[0].get(0);
        Ok(val.to_lowercase().starts_with("t"))
    }
}

#[async_trait]
impl DbClient for PgClientImpl {
    /// add user to router_table if not exists uaid
    async fn add_user(&self, user: &UserRecord) -> DbResult<()> {
        Ok(())
    }

    /// update user record in router_table at user.uaid
    async fn update_user(&self, user: &UserRecord) -> DbResult<()> {
        Ok(())
    }

    /// fetch user information from router_table for uaid.
    async fn get_user(&self, uaid: Uuid) -> DbResult<Option<UserRecord>> {
        Ok(None)
    }

    /// delete a user at uaid from router_table
    async fn remove_user(&self, uaid: Uuid) -> DbResult<()> {
        Ok(())
    }

    /// update list of channel_ids for uaid in meta table
    async fn add_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<()> {
        Ok(())
    }

    /// get all channels for uaid from meta table
    async fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>> {
        let empty = HashSet::new();

        Ok(empty)
    }

    /// remove an individual channel for a given uaid from meta table
    async fn remove_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<bool> {
        Ok(false)
    }

    /// remove node info for a uaid from router table
    async fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()> {
        Ok(())
    }

    /// write a message to message table
    async fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()> {
        Ok(())
    }

    /// remove a given message from the message table
    async fn remove_message(&self, uaid: Uuid, sort_key: String) -> DbResult<()> {
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
