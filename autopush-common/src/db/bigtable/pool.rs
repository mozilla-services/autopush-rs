use std::{fmt, sync::Arc, time::Duration};

use async_trait::async_trait;
use deadpool::managed::{Manager, PoolConfig, Timeouts};
use google_cloud_rust_raw::bigtable::v2::bigtable_grpc::BigtableClient;
use grpcio::{Channel, ChannelBuilder, ChannelCredentials, EnvBuilder};

use crate::db::bigtable::{BigTableDbSettings, BigTableError};
use crate::db::error::{DbError, DbResult};
use crate::db::DbSettings;

use super::bigtable_client::error;

/// The pool of BigTable Clients.
/// Note: BigTable uses HTTP/2 as the backbone, so the only really important bit
/// that we have control over is the "channel". For now, we're using the ClientManager to
/// create new Bigtable clients, which have channels associated with them.
/// The Manager also has the ability to return a channel, which is useful for
/// Bigtable administrative calls, which use their own channel.
#[derive(Clone)]
pub struct BigTablePool {
    /// Pool of db connections
    pub pool: deadpool::managed::Pool<BigtableClientManager>,
}

impl fmt::Debug for BigTablePool {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("SpannerDbPool").finish()
    }
}

/// Several convenience functions for using the pool.
impl BigTablePool {
    /// Get a new managed object from the pool.
    pub async fn get(
        &self,
    ) -> Result<deadpool::managed::Object<BigtableClientManager>, error::BigTableError> {
        self.pool
            .get()
            .await
            .map_err(|e| error::BigTableError::Pool(e.to_string()))
    }

    /// Get the pools manager, because we would like to talk to them.
    pub fn manager(&self) -> &BigtableClientManager {
        self.pool.manager()
    }
}

/// BigTable Pool Manager. This contains everything needed to create a new connection.
pub struct BigtableClientManager {
    settings: DbSettings,
    dsn: Option<String>,
    connection: String,
}

impl BigtableClientManager {
    fn new(
        settings: &DbSettings,
        dsn: Option<String>,
        connection: String,
    ) -> Result<Self, DbError> {
        Ok(Self {
            settings: settings.clone(),
            dsn,
            connection,
        })
    }
}

impl fmt::Debug for BigtableClientManager {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("deadpool::BtClientManager")
            .field("settings", &self.settings.clone())
            .finish()
    }
}

#[async_trait]
impl Manager for BigtableClientManager {
    type Error = DbError;
    type Type = BigtableClient;

    /// Create a new Bigtable Client with it's own channel.
    async fn create(&self) -> Result<BigtableClient, DbError> {
        debug!("🏊 Create a new pool entry.");
        let chan = Self::create_channel(self.dsn.clone())?.connect(self.connection.as_str());
        let client = BigtableClient::new(chan);
        Ok(client)
    }

    /// We can't really recycle a given client, so fail and the client should be dropped.
    async fn recycle(
        &self,
        _client: &mut Self::Type,
        _metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Self::Error> {
        debug!("🏊 Recycle requested.");
        Err(DbError::BTError(BigTableError::Admin("Recycle".to_owned())).into())
    }
}

impl BigtableClientManager {
    /// Get a new Channel, based on the application settings.
    pub fn get_channel(&self) -> Result<Channel, BigTableError> {
        Ok(Self::create_channel(self.dsn.clone())?.connect(self.connection.as_str()))
    }
    /// Channels are GRPCIO constructs that contain the actual command data paths.
    /// Channels seem to be fairly light weight.
    pub fn create_channel(dsn: Option<String>) -> Result<ChannelBuilder, BigTableError> {
        debug!("🏊 Creating new channel...");
        let env = Arc::new(EnvBuilder::new().build());
        let mut chan = ChannelBuilder::new(env)
            .max_send_message_len(1 << 28)
            .max_receive_message_len(1 << 28);
        // Don't get the credentials if we are running in the emulator
        if dsn
            .clone()
            .map(|v| v.contains("localhost"))
            .unwrap_or(false)
            || std::env::var("BIGTABLE_EMULATOR_HOST").is_ok()
        {
            debug!("🉑 Using emulator");
        } else {
            chan = chan.set_credentials(
                ChannelCredentials::google_default_credentials()
                    .map_err(|e| BigTableError::Admin(e.to_string()))?,
            );
            debug!("🉑 Using real");
        }
        Ok(chan)
    }
}

impl BigTablePool {
    /// Creates a new pool of BigTable db connections.
    pub fn new(settings: &DbSettings) -> DbResult<Self> {
        let endpoint = match &settings.dsn {
            Some(v) => v,
            None => {
                return Err(DbError::ConnectionError(
                    "No DSN specified in settings".to_owned(),
                ))
            }
        };
        let bt_settings = BigTableDbSettings::try_from(settings.db_settings.as_str())?;
        debug!("🉑 DSN: {}", &endpoint);
        // Url::parsed() doesn't know how to handle `grpc:` schema, so it returns "null".
        let parsed = url::Url::parse(endpoint).map_err(|e| {
            DbError::ConnectionError(format!("Invalid DSN: {:?} : {:?}", endpoint, e))
        })?;
        let origin = format!(
            "{}:{}",
            parsed
                .host_str()
                .ok_or_else(|| DbError::ConnectionError(format!(
                    "Invalid DSN: Unparsable host {:?}",
                    endpoint
                )))?,
            parsed.port().unwrap_or(8086)
        );
        if !parsed.path().is_empty() {
            return Err(DbError::ConnectionError(format!(
                "Invalid DSN: Table paths belong in settings : {:?}",
                endpoint
            )));
        }
        let connection = format!("{}{}", origin, parsed.path());
        debug!("🉑 connection string {}", &connection);

        // Construct a new manager and put them in a pool for handling future requests.
        let manager =
            BigtableClientManager::new(settings, settings.dsn.clone(), connection.clone())?;
        let mut config = PoolConfig::default();
        if let Some(size) = bt_settings.database_pool_max_size {
            debug!("🏊 Setting pool max size {}", &size);
            config.max_size = size as usize;
        };
        if let Some(timeout) = bt_settings.database_pool_connection_timeout {
            debug!("🏊 Setting connection timeout to {} milliseconds", &timeout);
            let timeouts = Timeouts {
                create: Some(Duration::from_millis(timeout as u64)),
                ..Default::default()
            };
            config.timeouts = timeouts;
        }
        let pool = deadpool::managed::Pool::builder(manager)
            .config(config)
            .build()
            .map_err(|e| DbError::BTError(BigTableError::Pool(e.to_string())))?;

        Ok(Self { pool })
    }
}
