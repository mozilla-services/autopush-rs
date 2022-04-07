use std::collections::HashSet;
use std::env;
use std::panic::panic_any;
use std::sync::Arc;

use cadence::StatsdClient;
use futures::{future, Future};
use futures_backoff::retry_if;
use postgres::{Client, Connection, Socket};

use crate::errors::*;
use crate::notification::Notification;
use crate::util::timing::sec_since_epoch;

use super::{CheckStorageResponse, HelloResponse, RegisterResponse, MAX_EXPIRY};
use super::{NotificationRecord, UserRecord};

pub struct PostgresStorage {
    db_client: Client,
    metrics: Arc<StatsdClient>,
    router_table_name: String,
    pub message_table_name: String,
    pub meta_table_name: String,
}

impl PostgresStorage {
    pub async fn from_opts(
        dsn: &str,
        message_table: &str,
        router_table: &str,
        meta_table: Option<&str>,
        metrics: Arc<StatsdClient>,
    ) -> std::result::Result<Self, Error> {
        Ok(Self {
            db_client: Self::connect(dsn).await?,
            metrics,
            router_table_name: router_table.to_owned(),
            message_table_name: message_table.to_owned(),
            meta_table_name: meta_table.unwrap_or(router_table).to_owned(),
        })
    }

    async fn connect(dsn: &str) -> std::result::Result<Client, Error> {
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

        let (client, connection) = Client(&pg_connect, postgres::NoTls)
            .await
            .unwrap();
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                panic_any(format!("PG Connection error {:?}", e));
            }
        });
        return Ok(client);
    }
}
