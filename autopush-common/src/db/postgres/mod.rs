/* Postgres DbClient implementation.
 * As noted elsewhere, autopush was originally designed to work with NoSql type databases.
 * This implementation was done partially as an experiment. Postgres allows for limited
 * NoSql-like functionality. The author, however, has VERY limited knowledge of postgres,
 * and there are likely many inefficiencies in this implementation.
 *
 * PRs are always welcome.
 */

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
#[cfg(feature = "reliable_report")]
use std::time::{Duration, SystemTime};

#[cfg(feature = "reliable_report")]
use chrono::TimeDelta;
use deadpool_postgres::{Pool, Runtime};
use serde_json::json;
use tokio_postgres::{types::ToSql, Row}; // Client is sync.

use async_trait::async_trait;
use cadence::StatsdClient;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::db::client::DbClient;
use crate::db::error::{DbError, DbResult};
use crate::db::{DbSettings, User};
use crate::notification::Notification;
use crate::{util, MAX_ROUTER_TTL_SECS};

use super::client::FetchMessageResponse;

#[cfg(feature = "reliable_report")]
const RELIABLE_LOG_TTL: TimeDelta = TimeDelta::days(60);

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostgresDbSettings {
    #[serde(default)]
    pub schema: Option<String>, // Optional DB Schema
    #[serde(default)]
    pub router_table: String, // Routing info
    #[serde(default)]
    pub message_table: String, // Message storage info
    #[serde(default)]
    pub meta_table: String, // Channels and meta info
    #[serde(default)]
    pub reliability_table: String, // Channels and meta info
    #[serde(default)]
    max_router_ttl: u64, // Max time for router records to live.
                         // #[serde(default)]
                         // pub use_tls: bool // Should you use a TLS connection to the db.
}

impl Default for PostgresDbSettings {
    fn default() -> Self {
        Self {
            schema: None,
            router_table: "router".to_owned(),
            message_table: "message".to_owned(),
            meta_table: "meta".to_owned(),
            reliability_table: "reliability".to_owned(),
            max_router_ttl: MAX_ROUTER_TTL_SECS,
            // use_tls: false,
        }
    }
}

impl TryFrom<&str> for PostgresDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        if setting_string.trim().is_empty() {
            return Ok(PostgresDbSettings::default());
        }
        serde_json::from_str(setting_string).map_err(|e| {
            DbError::General(format!(
                "Could not parse configuration db_settings: {:?}",
                e
            ))
        })
    }
}

#[derive(Clone)]
pub struct PgClientImpl {
    _metrics: Arc<StatsdClient>,
    db_settings: PostgresDbSettings,
    pool: Pool,
}

impl PgClientImpl {
    /// Create a new Postgres Client.
    ///
    /// This uses the `settings.db_dsn`. to try and connect to the postgres database.
    /// See https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html
    /// for parameter details and requirements.
    /// Example DSN: postgresql://user:password@host/database?option=val
    /// e.g. (postgresql://scott:tiger@dbhost/autopush?connect_timeout=10&keepalives_idle=3600)
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        let db_settings = PostgresDbSettings::try_from(settings.db_settings.as_ref())?;
        // TODO: If required, add the TlsConnect<Stream> wrapper here.
        let tls_flag = tokio_postgres::NoTls;
        if let Some(dsn) = settings.dsn.clone() {
            trace!("📮 Postgres Connect {}", &dsn);

            let pool = deadpool_postgres::Config {
                url: Some(dsn.clone()),
                ..Default::default()
            }
            .create_pool(Some(Runtime::Tokio1), tls_flag)
            .map_err(|e| DbError::General(e.to_string()))?;
            return Ok(Self {
                _metrics: metrics,
                db_settings,
                pool,
            });
        };
        Err(DbError::ConnectionError("No DSN specified".to_owned()))
    }

    /// Does the given table exist
    async fn table_exists(&self, table_name: String) -> DbResult<bool> {
        let (schema, table_name) = if table_name.contains('.') {
            let mut parts = table_name.splitn(2, '.');
            (
                parts.next().unwrap_or("public").to_owned(),
                // If we are in a situation where someone specified a table name as
                // `whatever.`, then we should absolutely panic here.
                parts.next().unwrap().to_owned(),
            )
        } else {
            ("public".to_owned(), table_name)
        };
        let rows = self
            .pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .query(
                "SELECT EXISTS (SELECT FROM pg_tables WHERE schemaname=$1 AND tablename=$2);",
                &[&schema, &table_name],
            )
            .await
            .map_err(DbError::PgError)?;
        let val: &str = rows[0].get(0);
        Ok(val.to_lowercase().starts_with('t'))
    }

    /// Return the router's expiration timestamp
    fn router_expiry(&self) -> u64 {
        util::sec_since_epoch() + self.db_settings.max_router_ttl
    }

    /// The router table contains how to route messages to the recipient UAID.
    pub(crate) fn router_table(&self) -> String {
        if let Some(schema) = &self.db_settings.schema {
            format!("{}.{}", schema, self.db_settings.router_table)
        } else {
            self.db_settings.router_table.clone()
        }
    }

    /// The message table contains stored messages for UAIDs.
    pub(crate) fn message_table(&self) -> String {
        if let Some(schema) = &self.db_settings.schema {
            format!("{}.{}", schema, self.db_settings.message_table)
        } else {
            self.db_settings.message_table.clone()
        }
    }

    /// The meta table contains channel and other metadata for UAIDs.
    /// With traditional "No-Sql" databases, this would be rolled into the
    /// router table.
    pub(crate) fn meta_table(&self) -> String {
        if let Some(schema) = &self.db_settings.schema {
            format!("{}.{}", schema, self.db_settings.meta_table)
        } else {
            self.db_settings.meta_table.clone()
        }
    }

    /// The reliability table contains message delivery reliability states.
    /// This is optional and should only be used to track internally generated
    /// and consumed messages based on the VAPID public key signature.
    #[cfg(feature = "reliable_report")]
    pub(crate) fn reliability_table(&self) -> String {
        if let Some(schema) = &self.db_settings.schema {
            format!("{}.{}", schema, self.db_settings.reliability_table)
        } else {
            self.db_settings.reliability_table.clone()
        }
    }
}

#[async_trait]
impl DbClient for PgClientImpl {
    /// add user to router_table if not exists uaid
    async fn add_user(&self, user: &User) -> DbResult<()> {
        self.pool.get().await.map_err(DbError::PgPoolError)?.execute(
            &format!("
            INSERT INTO {tablename} (uaid, connected_at, router_type, router_data, node_id, record_version, version, last_update, priv_channels, expiry)
             VALUES($1, $2::BIGINT, $3, $4, $5, $6::BIGINT, $7, $8::BIGINT, $9, $10::BIGINT)
             ON CONFLICT (uaid) DO
                UPDATE SET connected_at=EXCLUDED.connected_at,
                    router_type=EXCLUDED.router_type,
                    router_data=EXCLUDED.router_data,
                    node_id=EXCLUDED.node_id,
                    record_version=EXCLUDED.record_version,
                    version=EXCLUDED.version,
                    last_update=EXCLUDED.last_update,
                    priv_channels=EXCLUDED.priv_channels,
                    expiry=EXCLUDED.expiry
                    ;
            ", tablename=self.router_table()),
            &[&user.uaid.simple().to_string(),              // 1 
            &(user.connected_at as i64),                    // 2
            &user.router_type,                              // 3    
            &json!(user.router_data).to_string(),           // 4
            &user.node_id,                                  // 5
            &user.record_version.map(|i| i as i64),    // 6
            &(user.version.map(|v| v.simple().to_string())),   // 7
            &user.current_timestamp.map(|i| i as i64), // 8
            &user.priv_channels.iter().map(|v| v.to_string()).collect::<Vec<String>>(),
            &(self.router_expiry() as i64),                 // 10    
            ]
        ).await.map_err( DbError::PgError)?;
        Ok(())
    }

    /// update user record in router_table at user.uaid
    async fn update_user(&self, user: &mut User) -> DbResult<bool> {
        let cmd = format!(
            "UPDATE {tablename} SET connected_at=$2::BIGINT,
                router_type=$3,
                router_data=$4,
                node_id=$5,
                record_version=$6::BIGINT,
                version=$7,
                last_update=$8::BIGINT,
                priv_channels=$9,
                expiry=$10::BIGINT
            WHERE
                uaid = $1 AND connected_at < $2::BIGINT;
            ",
            tablename = self.router_table()
        );
        let result = self
            .pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .execute(
                &cmd,
                &[
                    &user.uaid.simple().to_string(),                 // 1
                    &(user.connected_at as i64),                     // 2
                    &user.router_type,                               // 3
                    &json!(user.router_data).to_string(),            //4
                    &user.node_id,                                   // 5
                    &(user.record_version.map(|i| i as i64)),        // 6
                    &(user.version.map(|v| v.simple().to_string())), // 7
                    &user.current_timestamp.map(|i| i as i64),       //8
                    &user
                        .priv_channels
                        .iter()
                        .map(|v| v.to_string())
                        .collect::<Vec<String>>(),
                    &(self.router_expiry() as i64), // 10
                ],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(result > 0)
    }

    /// fetch user information from router_table for uaid.
    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let row = self.pool.get().await.map_err(DbError::PgPoolError)?
        .query_opt(
            &format!(
                "SELECT connected_at, router_type, router_data, node_id, record_version, last_update, version, priv_channels 
                 FROM {tablename} 
                 WHERE uaid = $1",
                tablename=self.router_table()
            ),
            &[&uaid.simple().to_string()]
        )
        .await
        .map_err(DbError::PgError)?;

        let Some(row) = row else {
            return Ok(None);
        };

        // I was tempted to make this a From impl, but realized that it would mean making autopush-common require a dependency.
        // Maybe make this a deserialize?
        let priv_channels = if let Ok(Some(channels)) =
            row.try_get::<&str, Option<Vec<String>>>("priv_channels")
        {
            let mut priv_channels = HashSet::new();
            for channel in channels.iter() {
                let uuid = Uuid::from_str(channel).map_err(|e| DbError::General(e.to_string()))?;
                priv_channels.insert(uuid);
            }
            priv_channels
        } else {
            HashSet::new()
        };
        let resp = User {
            uaid: *uaid,
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
            node_id: row
                .try_get::<&str, Option<String>>("node_id")
                .map_err(DbError::PgError)?,
            record_version: row
                .try_get::<&str, Option<i64>>("record_version")
                .map_err(DbError::PgError)?
                .map(|v| v as u64),
            current_timestamp: row
                .try_get::<&str, Option<i64>>("last_update")
                .map_err(DbError::PgError)?
                .map(|v| v as u64),
            version: row
                .try_get::<&str, Option<String>>("version")
                .map_err(DbError::PgError)?
                // An invalid UUID here is a data integrity error.
                .map(|v| {
                    Uuid::from_str(&v).map_err(|e| {
                        DbError::Integrity("Invalid UUID found".to_owned(), Some(e.to_string()))
                    })
                })
                .transpose()?,
            priv_channels,
        };
        Ok(Some(resp))
    }

    /// delete a user at uaid from router_table
    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        self.pool
            .get()
            .await?
            .execute(
                &format!(
                    "DELETE FROM {tablename}
                     WHERE uaid = $1",
                    tablename = self.router_table()
                ),
                &[&uaid.simple().to_string()],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(())
    }

    /// update list of channel_ids for uaid in meta table
    /// Note: a conflicting channel_id is ignored, since it's already registered.
    /// This should probably be optimized into the router table as a set value,
    /// however I'm not familiar enough with Postgres to do so at this time.
    /// Channels can be somewhat ephemeral, and we also want to limit the potential of
    /// race conditions when adding or removing channels, particularly for mobile devices.
    /// For some efficiency (mostly around the mobile "daily refresh" call), I've broken
    /// the channels out by UAID into this table.
    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        self.pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .execute(
                &format!(
                    "INSERT 
                     INTO {tablename} (uaid, channel_id) VALUES ($1, $2) 
                     ON CONFLICT DO NOTHING",
                    tablename = self.meta_table()
                ),
                &[&uaid.simple().to_string(), &channel_id.simple().to_string()],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(())
    }

    /// Save all channels in a list
    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        if channels.is_empty() {
            trace!("📮 No channels to save.");
            return Ok(());
        };
        let uaid_str = uaid.simple().to_string();
        // TODO: REVISIT THIS!!! May be possible without the gross hack.
        // tokio-postgres doesn't store tuples as values, so you can't just construct
        // the query as `INSERT into ... (a, b) VALUES (?,?), (?,?)`
        // It does accept them as numerically specified values.
        // The following is a gross hack that does basically that.
        // The other option would be to just repeatedly call `self.add_channel()`
        // but that seems far worse.
        //
        // first, collect the values into a flat fector. We force the type in
        // the first item so that the second one is assumed.

        let mut params = Vec::<&(dyn ToSql + Sync)>::new();
        let mut new_channels = Vec::<String>::new();
        for channel in channels {
            new_channels.push(channel.simple().to_string());
        }

        // This is a bit cheesy, but we want capture the reference, not the value since
        // it will not live long enough. Clippy will complain that this is a needless
        // iterator, but it's not, really.
        #[allow(clippy::needless_range_loop)]
        for i in 0..new_channels.len() {
            params.push(&uaid_str);
            params.push(&new_channels[i]);
        }

        // Now construct the statement, iterate over the parameters we've got
        // and redistribute them into tuples.
        // (Remember, an existing channel_id is ignored during this insert since it's already registered)
        let statement = format!(
            "INSERT 
                INTO {tablename} (uaid, channel_id) 
                VALUES {vars} 
                ON CONFLICT DO NOTHING",
            tablename = self.meta_table(),
            // Postgres variables are 1-indexed.
            vars = Vec::from_iter((1..params.len() + 1).step_by(2).map(|v| format!(
                "(${}, ${})",
                v,
                v + 1
            )))
            .join(",")
        );
        // finally, do the insert.
        self.pool.get().await?.execute(&statement, &params).await?;
        Ok(())
    }

    /// get all channels for uaid from meta table
    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let mut result = HashSet::new();
        let rows = self
            .pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .query(
                &format!(
                    "SELECT distinct channel_id FROM {tablename} WHERE uaid = $1;",
                    tablename = self.meta_table()
                ),
                &[&uaid.simple().to_string()],
            )
            .await
            .map_err(DbError::PgError)?;
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
        let cmd = format!(
            "DELETE FROM {tablename} 
             WHERE uaid = $1 AND channel_id = $2;",
            tablename = self.meta_table()
        );
        let result = self
            .pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .execute(
                &cmd,
                &[&uaid.simple().to_string(), &channel_id.simple().to_string()],
            )
            .await?;
        // We sometimes want to know if the channel existed previously.
        Ok(result > 0)
    }

    /// remove node info for a uaid from router table
    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        node_id: &str,
        connected_at: u64,
        version: &Option<Uuid>,
    ) -> DbResult<bool> {
        let Some(version) = version else {
            return Err(DbError::General("Expected a user version field".to_owned()));
        };
        self.pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .execute(
                &format!(
                    "UPDATE {tablename} 
                        SET node_id = null 
                        WHERE uaid=$1 AND node_id = $2 AND connected_at = $3 AND version= $4;",
                    tablename = self.router_table()
                ),
                &[
                    &uaid.simple().to_string(),
                    &node_id,
                    &(connected_at as i64),
                    &version.simple().to_string(),
                ],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(true)
    }

    /// write a message to message table
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        // fun fact: serde_postgres exists, but only deserializes (as of 0.2)
        // (This is mutable if `reliable_report` enabled)
        #[allow(unused_mut)]
        let mut fields = vec![
            "uaid",
            "channel_id",
            "chid_message_id",
            "version",
            "ttl",
            "expiry",
            "topic",
            "timestamp",
            "data",
            "sortkey_timestamp",
            "headers",
        ];
        // (This is mutable if `reliable_report` enabled)
        #[allow(unused_mut)]
        let mut inputs = vec![
            "$1", "$2", "$3", "$4", "$5", "$6", "$7", "$8", "$9", "$10", "$11",
        ];
        #[cfg(feature = "reliable_report")]
        {
            fields.append(&mut ["reliability_id"].to_vec());
            inputs.append(&mut ["$12"].to_vec());
        }
        let cmd = format!(
            "INSERT INTO {tablename}
                ({fields})
                VALUES
                ({inputs}) ON CONFLICT (chid_message_id) DO UPDATE SET
                    uaid=EXCLUDED.uaid,
                    channel_id=EXCLUDED.channel_id,
                    version=EXCLUDED.version,
                    ttl=EXCLUDED.ttl,
                    expiry=EXCLUDED.expiry,
                    topic=EXCLUDED.topic,
                    timestamp=EXCLUDED.timestamp,
                    data=EXCLUDED.data,
                    sortkey_timestamp=EXCLUDED.sortkey_timestamp,
                    headers=EXCLUDED.headers",
            tablename = &self.message_table(),
            fields = fields.join(","),
            inputs = inputs.join(",")
        );
        self.pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .execute(
                &cmd,
                &[
                    &uaid.simple().to_string(),
                    &message.channel_id.simple().to_string(),
                    &message.chidmessageid(),
                    &message.version,
                    &(message.ttl as i64), // Postgres has no auto TTL.
                    &(util::sec_since_epoch() as i64 + message.ttl as i64),
                    &message.topic,
                    &(message.timestamp as i64),
                    &message.data.unwrap_or_default(),
                    &message.sortkey_timestamp.map(|v| v as i64),
                    &json!(message.headers).to_string(),
                    #[cfg(feature = "reliable_report")]
                    &message.reliability_id,
                ],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(())
    }

    /// remove a given message from the message table
    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        self.pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .execute(
                &format!(
                    "DELETE FROM {tablename} 
                     WHERE uaid=$1 AND chid_message_id = $2;",
                    tablename = self.message_table()
                ),
                &[&uaid.simple().to_string(), &(sort_key.to_owned())],
            )
            .await
            .map_err(DbError::PgError)?;

        Ok(())
    }

    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        for message in messages {
            self.save_message(uaid, message).await?;
        }
        Ok(())
    }

    /// fetch topic messages for the user up to {limit}
    /// Topic messages are auto-replacing singleton messages for a given user.
    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let messages: Vec<Notification> = self
            .pool
            .get()
            .await
            .map_err(DbError::PgPoolError)?
            .query(
                &format!(
                "SELECT channel_id, version, ttl, topic, timestamp, data, sortkey_timestamp, headers 
                 FROM {tablename} 
                 WHERE uaid=$1 
                 ORDER BY timestamp DESC 
                 LIMIT $2",
                tablename=&self.message_table(),
            ),
                &[&uaid.simple().to_string(), &(limit as i64)],
            )
            .await
            .map_err(DbError::PgError)?
            .iter()
            .map(|row: &Row| row.try_into())
            .collect::<Result<Vec<Notification>, DbError>>()?;

        if messages.is_empty() {
            Ok(Default::default())
        } else {
            Ok(FetchMessageResponse {
                timestamp: Some(messages[0].timestamp),
                messages,
            })
        }
    }

    /// Fetch messages for a user on or after a given timestamp up to {limit}
    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let uaid = uaid.simple().to_string();
        let response: Vec<Row> = if let Some(ts) = timestamp {
            trace!("📮 Fetching messages for user {} since {}", &uaid, ts);
            self.pool
                .get()
                .await
                .map_err(DbError::PgPoolError)?
                .query(
                    &format!(
                        "SELECT * FROM {} 
                         WHERE uaid = $1 AND timestamp > $2 AND expiry >= $3
                         ORDER BY timestamp 
                         LIMIT $3",
                        self.message_table()
                    ),
                    &[
                        &uaid,
                        &(ts as i64),
                        &(limit as i64),
                        &(util::sec_since_epoch() as i64),
                    ],
                )
                .await
        } else {
            trace!("📮 Fetching messages for user {}", &uaid);
            self.pool
                .get()
                .await
                .map_err(DbError::PgPoolError)?
                .query(
                    &format!(
                        "SELECT * 
                         FROM {} 
                         WHERE uaid = $1 
                         LIMIT $2",
                        self.message_table()
                    ),
                    &[&uaid, &(limit as i64)],
                )
                .await
        }?;

        let messages: Vec<Notification> = response
            .iter()
            .map(|row: &Row| row.try_into())
            .collect::<Result<Vec<Notification>, DbError>>()?;
        let timestamp = if !messages.is_empty() {
            Some(messages[0].timestamp)
        } else {
            None
        };

        Ok(FetchMessageResponse {
            timestamp,
            messages,
        })
    }

    /// Convenience function to check if the router table exists
    async fn router_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.router_table()).await
    }

    /// Convenience function to check if the message table exists
    async fn message_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.message_table()).await
    }

    #[cfg(feature = "reliable_report")]
    async fn log_report(
        &self,
        reliability_id: &str,
        new_state: crate::reliability::ReliabilityState,
    ) -> DbResult<()> {
        let timestamp =
            SystemTime::now() + Duration::from_secs(RELIABLE_LOG_TTL.num_seconds() as u64);
        debug!("📮 Logging report for {reliability_id} as {new_state}");
        /*
            INSERT INTO {tablename} (id, states) VALUES ({reliability_id}, json_build_object({state}, {timestamp}) )
            ON CONFLICT(id)
            UPDATE {tablename} SET states = jsonb_set(states, array[{state}], to_jsonb({timestamp}));
        */

        let tablename = &self.reliability_table();
        let state = new_state.to_string();
        self.pool
            .get()
            .await?
            .execute(
                &format!(
                "INSERT INTO {tablename} (id, states, last_update_timestamp) VALUES ($1, json_build_object($2, $3), $3)
                 ON CONFLICT (id) DO
                 UPDATE SET states = EXCLUDED.states, 
                 last_update_timestamp = EXCLUDED.last_update_timestamp;",
                    tablename = tablename
                ),
                &[&reliability_id, &state, &timestamp],
            )
            .await?;
        Ok(())
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        debug!("📮 Updating {uaid} current_timestamp:{timestamp}");
        let tablename = &self.router_table();

        trace!("📮 Purging {uaid} for < {timestamp}");
        let mut pool = self.pool.get().await.map_err(DbError::PgPoolError)?;
        let transaction = pool.transaction().await?;
        // Try to garbage collect old messages first.
        transaction
            .execute(
                &format!(
                    "DELETE FROM {} WHERE uaid = $1 and expiry < $2",
                    &self.message_table()
                ),
                &[
                    &uaid.simple().to_string(),
                    &(util::sec_since_epoch() as i64),
                ],
            )
            .await?;
        // Now, delete messages that we've already delivered.
        transaction
            .execute(
                &format!(
                    "DELETE FROM {} WHERE uaid = $1 AND timestamp IS NOT NULL AND timestamp < $2",
                    &self.message_table()
                ),
                &[&uaid.simple().to_string(), &(timestamp as i64)],
            )
            .await?;
        transaction.execute(
                &format!(
                    "UPDATE {tablename} SET last_update = $2::BIGINT, expiry= $3::BIGINT WHERE uaid = $1"
                ),
                &[
                    &uaid.simple().to_string(),
                    &(timestamp as i64),
                    &(self.router_expiry() as i64),
                ],
            )
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    fn name(&self) -> String {
        "Postgres".to_owned()
    }

    async fn health_check(&self) -> DbResult<bool> {
        // Replace this with a proper health check.
        let client = self.pool.get().await.map_err(DbError::PgPoolError);
        let row = client?.query_one("select true", &[]).await;
        Ok(!row?.is_empty())
    }

    /// Convenience function to return self as a Boxed DbClient
    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }
}

/* Note:
 * For preliminary testing, you will need to start a local postgres instance (see
 * https://www.docker.com/blog/how-to-use-the-postgres-docker-official-image/) and initialize the
 * database with `schema.psql`.
 * Once you have, you can define the environment variable `POSTGRES_HOST` to point to the
 * appropriate host (e.g. `postgres:post_pass@localhost:/autopush`). `new_client` will add the
 * `postgres://` prefix automatically.
 *
 * TODO: Really should move the bulk of the tests to a higher level and add backend specific
 * versions of `new_client`.
 *
 */
#[cfg(test)]
mod tests {
    use crate::util::sec_since_epoch;
    use crate::{logging::init_test_logging, util::ms_since_epoch};
    use rand::prelude::*;
    use serde_json::json;
    use std::env;

    use super::*;
    const TEST_CHID: &str = "DECAFBAD-0000-0000-0000-0123456789AB";
    const TOPIC_CHID: &str = "DECAFBAD-1111-0000-0000-0123456789AB";

    fn new_client() -> DbResult<PgClientImpl> {
        // Use an environment variable to potentially override the default storage test host.
        let host = env::var("POSTGRES_HOST").unwrap_or("localhost".into());
        let env_dsn = format!("postgres://{host}");
        debug!("📮 Connecting to {env_dsn}");
        let settings = DbSettings {
            dsn: Some(env_dsn),
            db_settings: json!(PostgresDbSettings {
                schema: Some("autopush".to_owned()),
                ..Default::default()
            })
            .to_string(),
        };
        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());
        PgClientImpl::new(metrics, &settings)
    }

    fn gen_test_user() -> String {
        // Create a semi-unique test user to avoid conflicting test values.
        let mut rng = rand::rng();
        let test_num = rng.random::<u8>();
        format!(
            "DEADBEEF-0000-0000-{:04}-{:012}",
            test_num,
            sec_since_epoch()
        )
    }

    #[actix_rt::test]
    async fn health_check() {
        let client = new_client().unwrap();

        let result = client.health_check().await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    /// Test if [increment_storage] correctly wipe expired messages
    #[actix_rt::test]
    async fn wipe_expired() -> DbResult<()> {
        init_test_logging();
        let client = new_client()?;

        let connected_at = ms_since_epoch();

        let uaid = Uuid::parse_str(&gen_test_user()).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();

        let node_id = "test_node".to_owned();

        // purge the user record if it exists.
        let _ = client.remove_user(&uaid).await;

        let test_user = User {
            uaid,
            router_type: "webpush".to_owned(),
            connected_at,
            router_data: None,
            node_id: Some(node_id.clone()),
            ..Default::default()
        };

        // purge the old user (if present)
        // in case a prior test failed for whatever reason.
        let _ = client.remove_user(&uaid).await;

        // can we add the user?
        let timestamp = sec_since_epoch();
        client.add_user(&test_user).await?;
        let test_notification = crate::db::Notification {
            channel_id: chid,
            version: "test".to_owned(),
            ttl: 1,
            timestamp,
            data: Some("Encrypted".into()),
            sortkey_timestamp: Some(timestamp),
            ..Default::default()
        };
        client.save_message(&uaid, test_notification).await?;
        client.increment_storage(&uaid, timestamp + 1).await?;
        let msgs = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_eq!(msgs.messages.len(), 0);
        assert!(client.remove_user(&uaid).await.is_ok());
        Ok(())
    }

    /// run a gauntlet of testing. These are a bit linear because they need
    /// to run in sequence.
    #[actix_rt::test]
    async fn run_gauntlet() -> DbResult<()> {
        init_test_logging();
        let client = new_client()?;

        let connected_at = ms_since_epoch();

        let user_id = &gen_test_user();
        let uaid = Uuid::parse_str(user_id).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let topic_chid = Uuid::parse_str(TOPIC_CHID).unwrap();

        let node_id = "test_node".to_owned();

        // purge the user record if it exists.
        let _ = client.remove_user(&uaid).await;

        let test_user = User {
            uaid,
            router_type: "webpush".to_owned(),
            connected_at,
            router_data: None,
            node_id: Some(node_id.clone()),
            ..Default::default()
        };

        // purge the old user (if present)
        // in case a prior test failed for whatever reason.
        let _ = client.remove_user(&uaid).await;

        // can we add the user?
        trace!("📮 Adding user {}", &user_id);
        client.add_user(&test_user).await?;
        let fetched = client.get_user(&uaid).await?;
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.router_type, "webpush".to_owned());

        // can we add channels?
        trace!("📮 Adding channel {} to user {}", &chid, &user_id);
        client.add_channel(&uaid, &chid).await?;
        let channels = client.get_channels(&uaid).await?;
        assert!(channels.contains(&chid));

        // can we add lots of channels?
        let mut new_channels: HashSet<Uuid> = HashSet::new();
        trace!("📮 Adding multiple channels to user {}", &user_id);
        new_channels.insert(chid);
        for _ in 1..10 {
            new_channels.insert(uuid::Uuid::new_v4());
        }
        let chid_to_remove = uuid::Uuid::new_v4();
        trace!(
            "📮 Adding removable channel {} to user {}",
            &chid_to_remove,
            &user_id
        );
        new_channels.insert(chid_to_remove);
        client.add_channels(&uaid, new_channels.clone()).await?;
        let channels = client.get_channels(&uaid).await?;
        assert_eq!(channels, new_channels);

        // can we remove a channel?
        trace!(
            "📮 Removing channel {} from user {}",
            &chid_to_remove,
            &user_id
        );
        assert!(client.remove_channel(&uaid, &chid_to_remove).await?);
        trace!(
            "📮 retrying Removing channel {} from user {}",
            &chid_to_remove,
            &user_id
        );
        assert!(!client.remove_channel(&uaid, &chid_to_remove).await?);
        new_channels.remove(&chid_to_remove);
        let channels = client.get_channels(&uaid).await?;
        assert_eq!(channels, new_channels);

        // now ensure that we can update a user that's after the time we set
        // prior. first ensure that we can't update a user that's before the
        // time we set prior to the last write
        let mut updated = User {
            connected_at,
            ..test_user.clone()
        };
        trace!(
            "📮 Attempting to update user {} with old connected_at: {}",
            &user_id,
            &updated.connected_at
        );
        let result = client.update_user(&mut updated).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // Make sure that the `connected_at` wasn't modified
        let fetched2 = client.get_user(&fetched.uaid).await?.unwrap();
        assert_eq!(fetched.connected_at, fetched2.connected_at);

        // and make sure we can update a record with a later connected_at time.
        let mut updated = User {
            connected_at: fetched.connected_at + 300,
            ..fetched2
        };
        trace!(
            "📮 Attempting to update user {} with new connected_at",
            &user_id
        );
        let result = client.update_user(&mut updated).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_ne!(
            fetched2.connected_at,
            client.get_user(&uaid).await?.unwrap().connected_at
        );
        // can we increment the storage for the user?
        trace!("📮 Incrementing storage timestamp for user {}", &user_id);
        client
            .increment_storage(&fetched.uaid, sec_since_epoch())
            .await?;

        let test_data = "An_encrypted_pile_of_crap".to_owned();
        let timestamp = sec_since_epoch();
        let sort_key = sec_since_epoch();
        let fetch_timestamp = timestamp;
        // Can we store a message?
        let test_notification = crate::db::Notification {
            channel_id: chid,
            version: "test".to_owned(),
            ttl: 300,
            timestamp,
            data: Some(test_data.clone()),
            sortkey_timestamp: Some(sort_key),
            ..Default::default()
        };
        trace!("📮 Saving message for user {}", &user_id);
        let res = client.save_message(&uaid, test_notification.clone()).await;
        assert!(res.is_ok());

        trace!("📮 Fetching all messages for user {}", &user_id);
        let mut fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_ne!(fetched.messages.len(), 0);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab all 1 of the messages that were submitted within the past 10 seconds.
        trace!(
            "📮 Fetching messages for user {} within the past 10 seconds",
            &user_id
        );
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(fetch_timestamp - 10), 999)
            .await?;
        assert_ne!(fetched.messages.len(), 0);

        // Try grabbing a message for 10 seconds from now.
        trace!(
            "📮 Fetching messages for user {} 10 seconds in the future",
            &user_id
        );
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(fetch_timestamp + 10), 999)
            .await?;
        assert_eq!(fetched.messages.len(), 0);

        // can we clean up our toys?
        trace!(
            "📮 Removing message for user {} :: {}",
            &user_id,
            &test_notification.chidmessageid()
        );
        assert!(client
            .remove_message(&uaid, &test_notification.chidmessageid())
            .await
            .is_ok());

        trace!("📮 Removing channel for user {}", &user_id);
        assert!(client.remove_channel(&uaid, &chid).await.is_ok());

        trace!("📮 Making sure no messages remain for user {}", &user_id);
        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        // Now, can we do all that with topic messages
        // Unlike bigtable, we don't use [fetch_topic_messages]: it always return None:
        // they are handled as usuals messages.
        client.add_channel(&uaid, &topic_chid).await?;
        let test_data = "An_encrypted_pile_of_crap_with_a_topic".to_owned();
        let timestamp = sec_since_epoch();
        let sort_key = sec_since_epoch();

        // We store 2 messages, with a single topic
        let test_notification_0 = crate::db::Notification {
            channel_id: topic_chid,
            version: "version0".to_owned(),
            ttl: 300,
            topic: Some("topic".to_owned()),
            timestamp,
            data: Some(test_data.clone()),
            sortkey_timestamp: Some(sort_key),
            ..Default::default()
        };
        assert!(client
            .save_message(&uaid, test_notification_0.clone())
            .await
            .is_ok());

        let test_notification = crate::db::Notification {
            timestamp: sec_since_epoch(),
            version: "version1".to_owned(),
            sortkey_timestamp: Some(sort_key + 10),
            ..test_notification_0
        };

        assert!(client
            .save_message(&uaid, test_notification.clone())
            .await
            .is_ok());

        let mut fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_eq!(fetched.messages.len(), 1);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab the message that was submitted.
        let fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_ne!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(client
            .remove_message(&uaid, &test_notification.chidmessageid())
            .await
            .is_ok());

        assert!(client.remove_channel(&uaid, &topic_chid).await.is_ok());

        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        let fetched = client.get_user(&uaid).await?.unwrap();
        assert!(client
            .remove_node_id(&uaid, &node_id, fetched.connected_at, &fetched.version)
            .await
            .is_ok());
        // did we remove it?
        let fetched = client.get_user(&uaid).await?.unwrap();
        assert_eq!(fetched.node_id, None);

        assert!(client.remove_user(&uaid).await.is_ok());

        assert!(client.get_user(&uaid).await?.is_none());
        Ok(())
    }

    #[actix_rt::test]
    async fn test_expiry() -> DbResult<()> {
        // Make sure that we really are purging messages correctly
        init_test_logging();
        let client = new_client()?;

        let uaid = Uuid::parse_str(&gen_test_user()).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let now = sec_since_epoch();

        let test_notification = crate::db::Notification {
            channel_id: chid,
            version: "test".to_owned(),
            ttl: 2,
            timestamp: now,
            data: Some("SomeData".into()),
            sortkey_timestamp: Some(now),
            ..Default::default()
        };
        client
            .add_user(&User {
                uaid,
                router_type: "test".to_owned(),
                connected_at: ms_since_epoch(),
                ..Default::default()
            })
            .await?;
        client.add_channel(&uaid, &chid).await?;
        debug!("🧪Writing test notif");
        client
            .save_message(&uaid, test_notification.clone())
            .await?;
        let key = uaid.simple().to_string();
        debug!("🧪Checking {}...", &key);
        let msg = client
            .fetch_timestamp_messages(&uaid, None, 1)
            .await?
            .messages
            .pop();
        assert!(msg.is_some());
        debug!("🧪Purging...");
        client.increment_storage(&uaid, now + 2).await?;
        debug!("🧪Checking for empty {}...", &key);
        let cc = client
            .fetch_timestamp_messages(&uaid, None, 1)
            .await?
            .messages
            .pop();
        assert!(cc.is_none());
        // clean up after the test.
        assert!(client.remove_user(&uaid).await.is_ok());
        Ok(())
    }
}
