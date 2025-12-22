use std::collections::{HashSet};
use std::time::{Duration, SystemTime};
use std::panic::panic_any;
use std::str::FromStr;
use std::sync::Arc;

use chrono::TimeDelta;
use serde_json::json;
use tokio_postgres::{types::ToSql, Client, NoTls}; // Client is sync.

use async_trait::async_trait;
use cadence::StatsdClient;
use serde::Deserialize;
use uuid::Uuid;

use crate::MAX_ROUTER_TTL_SECS;
use crate::db::client::DbClient;
use crate::db::error::{DbError, DbResult};
use crate::db::{
    CheckStorageResponse,DbSettings,USER_RECORD_VERSION, User
};
use crate::notification::Notification;

use super::client::FetchMessageResponse;
// use autopush_common::util::sec_since_epoch;

const RELIABLE_LOG_TTL:TimeDelta = TimeDelta::days(60);

#[derive(Debug, Clone, Deserialize)]
pub struct PostgresDbSettings {
    #[serde(default)]
    pub router_table: String,              // Routing info
    #[serde(default)]
    pub message_table: String,             // Message storage info
    #[serde(default)]
    pub meta_table: String,                // Channels and meta info
    #[serde(default)]
    pub reliability_table: String,                // Channels and meta info
    #[serde(default)]
    current_message_month: Option<String>, // For table rotation
    #[serde(default)]
    max_router_ttl: u64,           // Max time for router records to live.
}

impl Default for PostgresDbSettings {
    fn default() -> Self {
        Self {
            router_table: "router".to_owned(),
            message_table: "message".to_owned(),
            meta_table: "meta".to_owned(),
            reliability_table: "reliability".to_owned(),
            current_message_month: None,
            max_router_ttl: MAX_ROUTER_TTL_SECS,
        }
    }
}

impl TryFrom<&str> for PostgresDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(setting_string)
            .map_err(|e| DbError::General(format!("Could not parse DdbSettings: {:?}", e)))
    }
}

// TODO: Add a Pool manager?

#[allow(dead_code)] // TODO: Remove before flight
#[derive(Clone)]
pub struct PgClientImpl {
    db_client: Arc<Client>,
    _metrics: Arc<StatsdClient>,
    db_settings: PostgresDbSettings,
}

impl PgClientImpl {
    /// Create a new Postgres Client.
    ///
    /// This uses the `settings.db_dsn`. to try and connect to the postgres database.
    /// See https://docs.rs/tokio-postgres/latest/tokio_postgres/config/struct.Config.html
    /// for parameter details and requirements.
    /// Example DSN: postgresql://user:password@host/database?option=val
    /// e.g. (postgresql://scott:tiger@dbhost/autopush?connect_timeout=10&keepalives_idle=3600)
    pub async fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        if let Some(dsn) = settings.dsn.clone() {
            trace!("📬 Postgres Connect {}", &dsn);
            let (client, connection) = tokio_postgres::connect(&dsn, NoTls).await.map_err(|e| {
                DbError::ConnectionError(format!("Could not connect to postgres {:?}", e))
            })?;
            tokio::spawn(async move {
                if let Err(e) = connection.await {
                    panic_any(format!("PG Connection error {:?}", e));
                }
            });
            let db_settings = PostgresDbSettings::try_from(settings.db_settings.as_ref())?;
            return Ok(Self {
                db_client: Arc::new(client),
                _metrics: metrics,
                db_settings,
            });
        };
        Err(DbError::ConnectionError("No DSN specified".to_owned()))
    }

    /// Connect to the database.
    async fn connect(dsn: &str) -> DbResult<Client> {
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
            .db_client
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
    async fn add_user(&self, user: &User) -> DbResult<()> {
        self.db_client.execute(
            &format!("
            INSERT INTO {tablename} (uaid, connected_at, router_type, router_data, node_id, record_version, timestamp, priv_channels)
            VALUES($1, $2::INTEGER, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (uaid) DO
                UPDATE SET connected_at=EXCLUDED.connected_at,
                    router_type=EXCLUDED.router_type,
                    router_data=EXCLUDED.router_data,
                    node_id=EXCLUDED.node_id,
                    record_version=EXCLUDED.record_version,
                    timestamp=EXCLUDED.timestamp,
                    priv_channels=EXCLUDED.priv_channels,
                    ;
            ", tablename=self.db_settings.router_table),
            &[&user.uaid.simple().to_string(),   // TODO: Investigate serialization?
            &(user.connected_at as i64),
            &user.router_type,
            &json!(user.router_data).to_string(),
            &user.node_id,
            &(user.record_version.map(|i| i as i8)),
            &user.current_timestamp.map(|i| i as i64),
            &user.priv_channels.iter().map(|v| v.to_string()).collect::<Vec<String>>(),
            ]
        ).await.map_err( DbError::PgError)?;
        Ok(())
    }

    /// update user record in router_table at user.uaid
    async fn update_user(&self, user: &mut User) -> DbResult<bool> {
        self.db_client
            .execute(
                &format!(
                    "
            UPDATE {tablename} SET connected_at=$2,
                router_type=$3,
                router_data=$4,
                node_id=$5,
                record_version=$6,
                timestamp=$7,
                priv_channels=$8
            WHERE
                uaid = $1;
            ",
                    tablename = self.db_settings.router_table
                ),
                &[
                    &user.uaid.simple().to_string(),
                    &(user.connected_at as i64),
                    &user.router_type,
                    &json!(user.router_data).to_string(),
                    &user.node_id,
                    &(user.record_version.map(|i| i as i8)),
                    &user.current_timestamp.map(|i| i as i64),
                    &user.priv_channels.iter().map(|v| v.to_string()).collect::<Vec<String>>(),
                ],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(true)
    }

    /// fetch user information from router_table for uaid.
    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let row = self.db_client
        .query_one(
            &format!(
                "select connected_at, router_type, router_data, node_id, record_version, current_timestamp, version, priv_channels from {tablename} where uaid = ?",
                tablename=self.db_settings.router_table
            ),
            &[&uaid.simple().to_string()]
        )
        .await?;
        if row.is_empty() {
            return Ok(None);
        };

        // I was tempted to make this a From impl, but realized that it would mean making autopush-common require a dependency.
        // Maybe make this a deserialize?
        let priv_channels = if let Ok(Some(channels)) = row.try_get::<&str, Option<Vec<String>>>("priv_channels") {
            let mut priv_channels = HashSet::new();
            for channel in channels{
                let uuid = Uuid::from_str(&channel).map_err(|e| DbError::General(e.to_string()))?;
                priv_channels.insert(uuid);
            }
            priv_channels
        } else {
            HashSet::new()
        };
        let resp = User {
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
            node_id: row
                .try_get::<&str, Option<String>>("node_id")
                .map_err(DbError::PgError)?,
            record_version: row
                .try_get::<&str, Option<i64>>("record_version")
                .map_err(DbError::PgError)?
                .map(|v| v as u64),
            current_timestamp: row
                .try_get::<&str, Option<i64>>("timestamp")
                .map_err(DbError::PgError)?
                .map(|v| v as u64),
            version: row
                .try_get::<&str, Option<String>>("version")
                .map_err(DbError::PgError)?
                .map(|v| Uuid::from_str(&v).unwrap()),
            priv_channels: priv_channels,

        };
        Ok(Some(resp))
    }

    /// delete a user at uaid from router_table
    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        self.db_client
            .execute(
                &format!(
                    "DELETE FROM {tablename}
                WHERE uaid = $1",
                    tablename = self.db_settings.router_table
                ),
                &[&uaid.simple().to_string()],
            )
            .await
            .map_err(DbError::PgError)?;
        Ok(())
    }

    /// update list of channel_ids for uaid in meta table
    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let key = format!(
            "{}:{}",
            uaid.simple().to_string(),
            channel_id.simple().to_string()
        );
        self.db_client
            .execute(
                &format!(
                    "INSERT INTO {tablename} (uaid, uaid_channel_id) VALUES (?, ?);",
                    tablename = self.db_settings.meta_table
                ),
                &[&uaid.simple().to_string(), &key],
            )
            .await?;
        Ok(())
    }

    /// Save all channels in a list
    async fn add_channels(
        &self,
        uaid: &Uuid,
        channels: HashSet<Uuid>,
    ) -> DbResult<()> {
        if channels.is_empty() {
            trace!("No channels to save.");
            return Ok(());
        };
        let uaid_str = uaid.simple().to_string();
        // TODO: REVISIT THIS!!! May be possible without the gross hack.
        // tokio-postgres doesn't store tuples as values, so you can't just construct
        // the query as `INSERT into ... (a, b) VALUES (?,?), (?,?)`
        // It does accept them as numericly specified values.
        // The following is a gross hack that does basically that.
        // The other option would be to just repeatedly call `self.add_channel()`
        // but that seems far worse.
        //
        // first, collect the values into a flat fector. We force the type in
        // the first item so that the second one is assumed.

        let mut params = Vec::<&(dyn ToSql + Sync)>::new();
        let mut new_channels = Vec::<String>::new();
        for channel in channels {
            let channel_id = format!("{}:{}", &uaid_str, channel);
            new_channels.push(channel_id);
        }
        for i in 0..new_channels.len() {
            params.push(&uaid_str);
            params.push(&new_channels[i]);
        }

        // Now construct the statement, iterate over the parameters we've got
        // and redistribute them into tuples.
        let statement = format!(
            "INSERT INTO {tablename} (uaid, uaid_channel_id) VALUES {vars}",
            tablename = self.db_settings.meta_table,
            vars = Vec::from_iter((0..params.len()).step_by(2).map(|v| format!(
                "(${}, ${})",
                v,
                v + 1
            )))
            .join(",")
        );
        // finally, do the insert.
        self.db_client.execute(&statement, &params).await?;
        Ok(())
    }

        /// get all channels for uaid from meta table
    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let mut result = HashSet::new();
        let rows = self
            .db_client
            .query(
                &format!(
                    "SELECT uaid_channel_id FROM {tablename} WHERE uaid = ?;",
                    tablename = self.db_settings.meta_table
                ),
                &[&uaid.simple().to_string()],
            )
            .await?;
        for row in rows.iter() {
            // the primary key combination of uaid_channel_id is separated by a ':'
            // we only want the channelid, so strip the uaid element.
            let s = row
                .try_get::<&str, &str>("uaid_channel_id")
                .map(|v| v.split(':').last().unwrap_or(v))
                .map_err(DbError::PgError)?;
            result.insert(Uuid::from_str(s).map_err(|e| DbError::General(e.to_string()))?);
        }
        Ok(result)
    }

    /// remove an individual channel for a given uaid from meta table
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        Ok(self
            .db_client
            .execute(
                &format!(
                    "DELETE FROM {tablename} WHERE uaid_channel_id = ?;",
                    tablename = self.db_settings.meta_table
                ),
                &[&format!(
                    "{}:{}",
                    &uaid.simple().to_string(),
                    &channel_id.simple().to_string(),
                )],
            )
            .await
            .is_ok())
    }

    /// remove node info for a uaid from router table
    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64, version: &Option<Uuid>) -> DbResult<bool> {
        let Some(version) = version else {
            return Err(DbError::General("Expected a user version field".to_owned()));
        };
        self.db_client
        .execute(
            &format!("UPDATE {tablename} SET node_id = null WHERE uaid=? AND node_id = ? and connected_at = ? and version= ?;", tablename=self.db_settings.router_table),
            &[&uaid.simple().to_string(), &node_id, &(connected_at as i64), &version.to_string()]
        ).await?;
        Ok(true)
    }

    /// write a message to message table
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        // TODO: write serializer
        // fun fact: serde_postgres exists, but only deserializes (as of 0.2)
        let mut fields = vec!["uaid","channel_id","version","ttl","topic","timestamp",
        "data","sortkey_timestamp","headers"];
        let mut inputs = vec!["?","?","?","?","?","?","?","?","?"];
        #[cfg(feature = "reliable_report")]
        {
            fields.append(&mut ["reliability_id"].to_vec());
            inputs.append(&mut ["?"].to_vec());
        }

        self.db_client
            .execute(
                &format!(
                    "INSERT INTO {tablename}
                ({fields})
                VALUES
                ({inputs});
            ",
                    tablename = &self.db_settings.message_table,
                    fields = fields.join(","),
                    inputs = inputs.join(",")
                ),
                    &[&uaid.simple().to_string(),
                        &message.channel_id.simple().to_string(),
                        &message.version,
                        &message.ttl.to_string(), // Postgres has no auto TTL.
                        &message.topic.unwrap_or("".to_owned()),
                        &message.timestamp.to_string(),
                        &message.data.unwrap_or("".to_owned()),
                        &message.sortkey_timestamp.map(|v| v.to_string()).unwrap_or("".to_owned()),
                        &json!(message.headers).to_string(),
                        #[cfg(feature = "reliable_report")]
                        &message.reliability_id.unwrap_or("".to_owned())
                    ]
            )
            .await?;
        Ok(())
    }

    /// remove a given message from the message table
    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        self.db_client
            .execute(
                &format!(
                    "DELETE FROM {tablename} WHERE uaid=? AND chid_message_id = ?;",
                    tablename = self.db_settings.message_table
                ),
                &[&uaid.simple().to_string(), &sort_key],
            )
            .await?;

        Ok(())
    }

    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        for message in messages {
            self.save_message(uaid, message).await?;
        };
        Ok(())
    }

    /// fetch topic messages for the user up to {limit}
    /// Topic messages are auto-replacing singleton messages for a given user.
    async fn fetch_topic_messages(&self, uaid: &Uuid, limit: usize) -> DbResult<FetchMessageResponse> {
        let messages: Vec<Notification> = self
            .db_client
            .query(
                &format!(
                "SELECT channel_id, version, ttl, topic, timestamp, data, sortkey_timestamp, headers FROM {tablename} WHERE uaid=? LIMIT ? ORDER BY timestamp DESC", // TODO: Check the timestamp DESC here!
                tablename=&self.db_settings.message_table,
            ),
                &[&uaid.simple().to_string(), &(limit as i64)],
            )
            .await?
            .iter()
            // TODO: add converters from tokio_postgres::Row to Notification (see prior code?)
            .map(|row| row.into())
            .collect();

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
        let response = if let Some(ts) = timestamp {
            self.db_client
                .query(
                    &format!(
                        "SELECT * FROM {} WHERE uaid = ? and timestamp > ? limit ?",
                        self.db_settings.message_table
                    ),
                    &[&uaid, &(ts as i64), &(limit as i64)],
                )
                .await
        } else {
            self.db_client
                .query(
                    &format!(
                        "SELECT * FROM {} WHERE uaid = ? limit ?",
                        self.db_settings.message_table
                    ),
                    &[&uaid, &(limit as i64)],
                )
                .await
        }?;

        let messages: Vec<Notification> = response.iter().map(|row| row.into()).collect();
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
        self.table_exists(self.db_settings.router_table.clone())
            .await
    }

    /// Convenience function to check if the message table exists
    async fn message_table_exists(&self) -> DbResult<bool> {
        self.table_exists(self.db_settings.message_table.clone())
            .await
    }

    #[cfg(feature="reliable_report")]
    async fn log_report(
        &self,
        reliability_id: &str,
        new_state: crate::reliability::ReliabilityState,
    ) -> DbResult<()> {
        let timestamp = SystemTime::now() + Duration::from_secs(RELIABLE_LOG_TTL.num_seconds() as u64);
        debug!("📬 Logging report for {reliability_id} as {new_state}");
        /* 
            INSERT INTO {tablename} (id, states) VALUES ({reliability_id}, json_build_object({state}, {timestamp}) )
            ON CONFLICT(id)
            UPDATE {tablename} SET states = jsonb_set(states, array[{state}], to_jsonb({timestamp}));
        */

        let tablename = &self.db_settings.reliability_table;
        let state = new_state.to_string();
        self.db_client.execute(&format!("INSERT INTO {tablename} (id, states) VALUES ($1, json_build_object($2, $3) )
            ON CONFLICT(id)
            UPDATE {tablename} SET states = jsonb_set(states, array[$2], to_jsonb($3))"),
            &[
                &state,
                &timestamp,
            ]
        ).await?;
        Ok(())
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        debug!("📬 Updating {uaid} current_timestamp:{timestamp}");
        let tablename = &self.db_settings.router_table;

        self.db_client.execute(&format!("UPDATE {tablename} SET current_timestamp = $2 where uaid = $1"), &[&uaid.to_string(), &(timestamp as i64)]).await?;

        Ok(())
    }

    fn name(&self) -> String {
        "Postgres".to_owned()
    }

    async fn health_check(&self) -> DbResult<bool> {
        // Replace this with a proper health check.
        let row =self.db_client.query_one("select true", &[]).await?;
        Ok(!row.is_empty())
    }

    /// Convenience function to return self as a Boxed DbClient
    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }

}
