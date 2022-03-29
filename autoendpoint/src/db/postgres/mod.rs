//TODO: REMOVE:
#[allow(unused_imports)]
use std::collections::HashSet;
use std::panic::panic_any;
use std::str::FromStr;
use std::sync::Arc;

use tokio_postgres::Client; // Client is sync.

use async_trait::async_trait;
use cadence::StatsdClient;
use serde_json::json;
use uuid::Uuid;

use crate::db::client::DbClient;
use crate::db::error::{DbError, DbResult};
use crate::settings::Settings;
use autopush_common::db::UserRecord;
use autopush_common::notification::Notification;
// use autopush_common::util::sec_since_epoch;

#[derive(Clone)]
pub struct PgClientImpl {
    client: Arc<Client>,
    _metrics: Arc<StatsdClient>,
    router_table: String,  // Routing information
    message_table: String, // Message storage information
    meta_table: String,    // Channels and meta info for a user.
}

impl PgClientImpl {
    /// We'll use this eventually...
    #[allow(dead_code)]
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
                _metrics: metrics,
                router_table: settings.router_table_name,
                message_table: settings.message_table_name,
                meta_table: settings
                    .meta_table_name
                    .unwrap_or_else(|| "meta".to_owned()),
            });
        };
        Err(DbError::Connection("No DSN specified".to_owned()))
    }

    async fn table_exists(&self, table_name: String) -> DbResult<bool> {
        let rows = self
            .client
            .query(
                &format!("SELECT EXISTS (SELECT FROM pg_tables where schemaname='public' AND tablename={tablename});", tablename=table_name),
                &[],
            )
            .await?;
        let val: &str = rows[0].get(0);
        Ok(val.to_lowercase().starts_with('t'))
    }
}

#[async_trait]
impl DbClient for PgClientImpl {
    /// add user to router_table if not exists uaid
    async fn add_user(&self, user: &UserRecord) -> DbResult<()> {
        self.client.execute(
            &format!("
            INSERT INTO {tablename}(uaid, connected_at, router_type, router_data, last_connect, node_id, record_version, current_month)
            VALUES($1, $2::INTEGER, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (uaid) DO
                UPDATE SET connected_at=EXCLUDED.connected_at,
                    router_type=EXCLUDED.router_type,
                    router_data=EXCLUDED.router_data,
                    last_connect=EXCLUDED.last_connect,
                    node_id=EXCLUDED.node_id,
                    record_version=EXCLUDED.record_version,
                    current_month=EXCLUDED.current_month;
            ", tablename=self.router_table),
            &[&user.uaid.to_simple().to_string(),   // TODO: Investigate serialization?
            &(user.connected_at as i64),
            &user.router_type,
            &json!(user.router_data).to_string(),
            &(user.last_connect.map(|i| i as i64)),
            &user.node_id,
            &(user.record_version.map(|i| i as i8)),
            &user.current_month]
        ).await.map_err( DbError::Postgres)?;
        Ok(())
    }

    /// update user record in router_table at user.uaid
    async fn update_user(&self, user: &UserRecord) -> DbResult<()> {
        self.client
            .execute(
                &format!(
                    "
            UPDATE {tablename} SET connected_at=$2,
                router_type=$3,
                router_data=$4,
                last_connect=$5,
                node_id=$6,
                record_version=$7,
                current_month=$8
            WHERE
                uaid = $1;
            ",
                    tablename = self.router_table
                ),
                &[
                    &user.uaid.to_simple().to_string(),
                    &(user.connected_at as i64),
                    &user.router_type,
                    &json!(user.router_data).to_string(),
                    &(user.last_connect.map(|i| i as i64)),
                    &user.node_id,
                    &(user.record_version.map(|i| i as i8)),
                    &user.current_month,
                ],
            )
            .await
            .map_err(DbError::Postgres)?;
        Ok(())
    }

    /// fetch user information from router_table for uaid.
    async fn get_user(&self, uaid: Uuid) -> DbResult<Option<UserRecord>> {
        let row = self.client.query_one(
            &format!(
                "select connected_at, router_type, router_data, last_connect, node_id, record_version, current_month from {tablename} where uaid = ?",
                tablename=self.router_table
            ),
            &[&uaid.to_simple().to_string()]
        )
        .await?;
        if row.is_empty() {
            return Ok(None);
        };
        // I was tempted to make this a From impl, but realized that it would mean making autopush-common require a dependency.
        // Maybe make this a deserialize?
        let resp = UserRecord {
            uaid,
            connected_at: row
                .try_get::<&str, i64>("connected_at")
                .map_err(DbError::Postgres)? as u64,
            router_type: row
                .try_get::<&str, String>("router_type")
                .map_err(DbError::Postgres)?,
            router_data: serde_json::from_str(
                row.try_get::<&str, &str>("router_data")
                    .map_err(DbError::Postgres)?,
            )
            .map_err(|e| DbError::General(e.to_string()))?,
            last_connect: row
                .try_get::<&str, Option<i64>>("last_connect")
                .map_err(DbError::Postgres)?
                .map(|v| v as u64),
            node_id: row
                .try_get::<&str, Option<String>>("node_id")
                .map_err(DbError::Postgres)?,
            record_version: row
                .try_get::<&str, Option<i8>>("record_verison")
                .map_err(DbError::Postgres)?
                .map(|v| v as u8),
            current_month: row
                .try_get::<&str, Option<String>>("current_month")
                .map_err(DbError::Postgres)?,
        };
        Ok(Some(resp))
    }

    /// delete a user at uaid from router_table
    async fn remove_user(&self, uaid: Uuid) -> DbResult<()> {
        self.client
            .execute(
                &format!(
                    "DELETE FROM {tablename}
                WHERE uaid = $1",
                    tablename = self.router_table
                ),
                &[&uaid.to_simple().to_string()],
            )
            .await
            .map_err(DbError::Postgres)?;
        Ok(())
    }

    /// update list of channel_ids for uaid in meta table
    async fn add_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<()> {
        self.client
            .execute(
                &format!(
                    "INSERT INTO {tablename} (uaid, channel_id) VALUES (?, ?);",
                    tablename = self.meta_table
                ),
                &[
                    &uaid.to_simple().to_string(),
                    &channel_id.to_simple().to_string(),
                ],
            )
            .await?;
        Ok(())
    }

    /// get all channels for uaid from meta table
    async fn get_channels(&self, uaid: Uuid) -> DbResult<HashSet<Uuid>> {
        let mut result = HashSet::new();
        let rows = self
            .client
            .query(
                &format!(
                    "SELECT channel_id FROM {tablename} WHERE uaid = ?;",
                    tablename = self.meta_table
                ),
                &[&uaid.to_simple().to_string()],
            )
            .await?;
        for row in rows.iter() {
            let s = row
                .try_get::<&str, &str>("channel_id")
                .map_err(DbError::Postgres)?;
            result.insert(Uuid::from_str(s).map_err(|e| DbError::General(e.to_string()))?);
        }
        Ok(result)
    }

    /// remove an individual channel for a given uaid from meta table
    async fn remove_channel(&self, uaid: Uuid, channel_id: Uuid) -> DbResult<bool> {
        Ok(self
            .client
            .execute(
                &format!(
                    "DELETE FROM {tablename} WHERE uaid=? AND channel_id = ?;",
                    tablename = self.meta_table
                ),
                &[
                    &uaid.to_simple().to_string(),
                    &channel_id.to_simple().to_string(),
                ],
            )
            .await
            .is_ok())
    }

    /// remove node info for a uaid from router table
    async fn remove_node_id(&self, uaid: Uuid, node_id: String, connected_at: u64) -> DbResult<()> {
        self.client.execute(
            &format!("UPDATE {tablename} SET node_id = null WHERE uaid=? AND node_id = ? and connected_at = ?;", tablename=self.router_table),
            &[&uaid.to_simple().to_string(), &node_id, &(connected_at as i64)]
        ).await?;
        Ok(())
    }

    /// write a message to message table
    async fn save_message(&self, uaid: Uuid, message: Notification) -> DbResult<()> {
        // TODO: write serializer
        // fun fact: serde_postgres exists, but only deserializes (as of 0.2)

        self.client
            .execute(
                &format!(
                    "INSERT INTO {tablename}
                (uaid, channel_id, version, ttl, topic, timestamp, data, sortkey_timestamp, headers)
                VALUES
                (?, ?, ?, ?, ?, ?, ?, ?, ?);
            ",
                    tablename = &self.message_table
                ),
                &[
                    &uaid.to_simple().to_string(),
                    &message.channel_id.to_simple().to_string(),
                    &message.version,
                    &(message.ttl as i64), // Postgres has no auto TTL.
                    &message.topic,
                    &(message.timestamp as i64),
                    &message.data,
                    &message.sortkey_timestamp.map(|v| v as i64),
                    &json!(message.headers).to_string(),
                ],
            )
            .await?;
        Ok(())
    }

    /// remove a given message from the message table
    async fn remove_message(&self, uaid: Uuid, sort_key: String) -> DbResult<()> {
        self.client
            .execute(
                &format!(
                    "DELETE FROM {tablename} WHERE uaid=? AND chid_message_id = ?;",
                    tablename = self.message_table
                ),
                &[&uaid.to_simple().to_string(), &sort_key],
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
