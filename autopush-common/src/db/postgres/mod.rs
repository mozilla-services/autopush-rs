/// This is very much a work-in-progress, and still needs impl DbStorageClient
///
use std::panic::panic_any;
use std::sync::Arc;

use cadence::StatsdClient;
use tokio_postgres::Client;

use crate::errors::ApiResult;

#[allow(dead_code)] // TODO: Remove before flight
pub struct PostgresStorage {
    db_client: Client,     // temporarily silence
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
    ) -> ApiResult<Self> {
        Ok(Self {
            db_client: Self::connect(dsn).await?,
            metrics,
            router_table_name: router_table.to_owned(),
            message_table_name: message_table.to_owned(),
            meta_table_name: meta_table.unwrap_or(router_table).to_owned(),
        })
    }

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
}

/* TODO

impl DbStorageClient for PostgresStorage {
    async fn hello(
        &self,
        connected_at: u64,
        uaid: Option<&Uuid>,
        router_url: &str,
        defer_registration: bool,
    ) -> ApiResult<HelloResponse>{

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

    async fn get_user(&self, uaid: &Uuid) -> ApiResult<Option<UserRecord>>{

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
*/
