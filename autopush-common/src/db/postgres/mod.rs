use std::collections::HashSet;
use std::panic::panic_any;
use std::str::FromStr;
use std::sync::Arc;

use tokio_postgres::{Client, NoTls}; // Client is sync.
use serde_json::json;

use async_trait::async_trait;
use cadence::StatsdClient;
use crate::db::client::DbClient;
use uuid::Uuid;

use crate::db::{DbCommandClient, DbSettings, HelloResponse, UserRecord, USER_RECORD_VERSION};
use crate::db::error::{DbError, DbResult};
use crate::errors::ApiResult;
use crate::notification::Notification;
// use autopush_common::util::sec_since_epoch;


#[allow(dead_code)] // TODO: Remove before flight
#[derive(Clone)]
pub struct PostgresStorage {
    client: Arc<Client>,
    _metrics: Arc<StatsdClient>,
    router_table: String,  // Routing information
    message_table: String, // Message storage information
    meta_table: String,    // Channels and meta info for a user.
    current_message_month: Option<String> // For table rotation
}

impl PostgresStorage {
    /// Create a new Postgres Client.
    ///
    /// This uses the `settings.db_dsn`. to try and connect to the postgres database.
    /// See https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html
    /// for parameter details and requirements.
    /// Example DSN: postgresql://user:password@host/database?option=val
    /// e.g. (postgresql://scott:tiger@dbhost/autopush?connect_timeout=10&keepalives_idle=3600)
    pub async fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        if let Some(dsn) = settings.dsn.clone() {
            trace!("Postgres Connect {}", &dsn);
            let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.map_err(|e| {
                DbError::ConnectionError(format!("Could not connect to postgres {:?}", e))
            })?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    panic_any(format!("PG Connection error {:?}", e));
                }
            });
            return Ok(Self {
                client: Arc::new(client),
                _metrics: metrics,
                router_table: settings.router_tablename.clone(),
                message_table: settings.message_tablename.clone(),
                meta_table: settings
                    .meta_tablename
                    .clone()
                    .unwrap_or_else(|| "meta".to_owned()),
                current_message_month: None,
            });
        };
        Err(DbError::ConnectionError("No DSN specified".to_owned()))
    }

    async fn current_message_month(_client: &Client) -> Option<String> {
        // TODO
        None
    }

    /// Connect to the database.
    async fn connect(dsn: &str) -> ApiResult<Client> {
        let parsed = url::Url::parse(dsn).unwrap(); // TODO: FIX ERRORS!!
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
            .unwrap();
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                panic_any(format!("PG Connection error {:?}", e));
            }
        });
        return Ok(client);
    }


    /// Does the given table exist
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
impl DbClient for PostgresStorage {
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
        ).await.map_err( DbError::PgError)?;
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
            .map_err(DbError::PgError)?;
        Ok(())
    }

    /// fetch user information from router_table for uaid.
    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<UserRecord>> {
        let row = self.client
        .query_one(
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
            uaid: uaid.clone(),
            connected_at: row
                .try_get::<&str, i64>("connected_at")
                .map_err(DbError::PgError)? as u64,
            router_type: row
                .try_get::<&str, String>("router_type")
                .map_err(DbError::PgError)?,
            router_data: serde_json::from_str(
                row.try_get::<&str, &str>("router_data")
                    .map_err(DbError::PgError)?,
            )
            .map_err(|e| DbError::General(e.to_string()))?,
            last_connect: row
                .try_get::<&str, Option<i64>>("last_connect")
                .map_err(DbError::PgError)?
                .map(|v| v as u64),
            node_id: row
                .try_get::<&str, Option<String>>("node_id")
                .map_err(DbError::PgError)?,
            record_version: row
                .try_get::<&str, Option<i8>>("record_verison")
                .map_err(DbError::PgError)?
                .map(|v| v as u8),
            current_month: row
                .try_get::<&str, Option<String>>("current_month")
                .map_err(DbError::PgError)?,
        };
        Ok(Some(resp))
    }

    /// delete a user at uaid from router_table
    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
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
            .map_err(DbError::PgError)?;
        Ok(())
    }

    /// update list of channel_ids for uaid in meta table
    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
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
    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
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
                .map_err(DbError::PgError)?;
            result.insert(Uuid::from_str(s).map_err(|e| DbError::General(e.to_string()))?);
        }
        Ok(result)
    }

    /// remove an individual channel for a given uaid from meta table
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
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
    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        self.client
        .execute(
            &format!("UPDATE {tablename} SET node_id = null WHERE uaid=? AND node_id = ? and connected_at = ?;", tablename=self.router_table),
            &[&uaid.to_simple().to_string(), &node_id, &(connected_at as i64)]
        ).await?;
        Ok(())
    }

    /// write a message to message table
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
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
    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
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

    /// Convenience function to check if the router table exists
    async fn router_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.router_table.clone()).await
    }

    /// Convenience function to check if the message table exists
    async fn message_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.message_table.clone()).await
    }

    /// Convenience function for the message table name
    fn message_table(&self) -> &str {
        trace!("pg message table {:?}", &self.message_table);
        &self.message_table
    }

    /// Convenience function to return self as a Boxed DbClient
    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }
}


/// Higher level command handler for Postgres Storage.
struct DbPgHandler{
    client: PostgresStorage,
}

impl DbPgHandler {
    async fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> ApiResult<Self> {
        Ok(Self{
            client: PostgresStorage::new(metrics, settings).await?,
        })
    }
}

//* TODO
#[async_trait]
impl DbCommandClient for DbPgHandler
{

    /// Handle the initial "HELLO" message type.
    ///
    /// Fetch and check the existing UAID
    async fn hello(
        &self,
        connected_at: u64,
        uaid: Option<&Uuid>,
        router_url: &str,
        defer_registration: bool,
    ) -> ApiResult<HelloResponse>{

        trace!(
            "### uaid {:?}, defer_registration: {:?}",
            &uaid,
            &defer_registration,
        );
        let mut user = if let Some(uaid) = uaid {
            self.client.get_user(
                uaid,
            ).await?.map(|mut user| {
                // update the connected at timestamp
                user.connected_at = connected_at;
                user
            }
            )
        } else {
            None
        }.unwrap_or_else(|| UserRecord{
            current_month: self.client.current_message_month,
            node_id: Some(router_url.to_owned()),
            connected_at,
            ..Default::default()
        });

        {
            // check_user()
            // check the user record to see if it's still valid.
            // this is mostly required for table rotation
            // TODO: Check if resp.current_month in list of valid message months. (else return errno 105)
            // TODO: If user_month != current {rotate_message_table}
            // TODO: reset the UAID if resp.record_version < USER_RECORD_VERSION
        }

        Ok(HelloResponse{
                    connected_at,
                    message_month: self.client.current_message_month.unwrap_or_default(),
                    check_storage: true,
                    rotate_message_table: if let Some(month) = self.client.current_message_month {
                        user.current_month.unwrap_or_default() == month
                    } else {
                        false
                    },
                    uaid : Some(user.uaid),
                    reset_uaid: user.record_version.unwrap_or(0) < USER_RECORD_VERSION,
                    deferred_user_registration: if defer_registration {
                        // Wait 'til later to register this user.
                        Some(user)
                    } else {
                        // Register the new user now.
                        self.client.add_user(&user).await?;
                        None
                    }
                })
    }

    async fn register(
        &self,
        uaid: &Uuid,
        channel_id: &Uuid,
        message_month: &str,
        endpoint: &str,
        register_user: Option<&UserRecord>,
    ) -> ApiResult<RegisterResponse>{

    }

    async fn drop_uaid(&self, uaid: &Uuid) -> ApiResult<()>{

    }

    async fn unregister(
        &self,
        uaid: &Uuid,
        channel_id: &Uuid,
        message_month: &str,
    ) -> ApiResult<bool>{

    }

    async fn migrate_user(&self, uaid: &Uuid, message_month: &str) -> ApiResult<()>{

    }

    async fn store_message(
        &self,
        uaid: &Uuid,
        message_month: String,
        message: Notification,
    ) -> ApiResult<()>{

    }

    async fn store_messages(
        &self,
        uaid: &Uuid,
        message_month: &str,
        messages: Vec<Notification>,
    ) -> ApiResult<()>{

    }

    async fn delete_message(
        &self,
        table_name: &str,
        uaid: &Uuid,
        notif: &Notification,
    ) -> ApiResult<()>{

    }

    async fn check_storage(
        &self,
        table_name: &str,
        uaid: &Uuid,
        include_topic: bool,
        timestamp: Option<u64>,
    ) -> ApiResult<CheckStorageResponse>{

    }

    async fn get_user_channels(&self, uaid: &Uuid, message_table: &str)
        -> ApiResult<HashSet<Uuid>>{

        }

    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        node_id: String,
        connected_at: u64,
    ) -> ApiResult<()>{

    }

}
// */
