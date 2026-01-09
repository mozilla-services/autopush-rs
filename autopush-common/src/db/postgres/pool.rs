use std::{
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use actix_web::rt;
use cadence::StatsdClient;
use deadpool::managed::{Manager, PoolConfig, PoolError, QueueMode, RecycleError, Timeouts};

use crate::db::error::{DbError, DbResult};
use crate::db::DbSettings;

const MAX_MESSAGE_LEN: i32 = 1 << 28; // 268,435,456 bytes
const DEFAULT_POSTGRES_PORT: u16 = 5432;

/// The pool of Postgres Clients.
#[derive(Clone)]
pub struct PostgresPool {
    /// Pool of db connections
    pub pool: deadpool::managed::Pool<PostgresClientManager>,
    _metrics: Arc<StatsdClient>,
}

impl fmt::Debug for PostgresPool {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("PostgresPool").finish()
    }
}

/// Several convenience functions for using the pool.
impl PostgresPool {
    /// Get a new managed object from the pool.
    pub async fn get(
        &self,
    ) -> Result<deadpool::managed::Object<PostgresClientManager>, PostgresError> {
        let obj = self.pool.get().await.map_err(|e| match e {
            PoolError::Timeout(tt) => PostgresError::PoolTimeout(tt),
            PoolError::Backend(e) => e,
            e => PostgresError::Pool(Box::new(e)),
        })?;
        debug!("📮 Got db from pool");
        Ok(obj)
    }

    /// Get the pools manager, because we would like to talk to them.
    pub fn get_channel(&self) -> Result<Channel, PostgresError> {
        self.pool.manager().get_channel()
    }

    /// Creates a new pool of Postgres db connections.
    pub fn new(settings: &DbSettings, metrics: &Arc<StatsdClient>) -> DbResult<Self> {
        let Some(endpoint) = &settings.dsn else {
            return Err(DbError::ConnectionError(
                "No DSN specified in settings".to_owned(),
            ));
        };
        let bt_settings = PostgresDbSettings::try_from(settings.db_settings.as_str())?;
        debug!("📮 DSN: {}", &endpoint);
        // Url::parsed() doesn't know how to handle `postgres:` schema, so it returns "null".
        let parsed = url::Url::parse(endpoint)
            .map_err(|e| DbError::ConnectionError(format!("Invalid DSN: {endpoint:?} : {e:?}")))?;
        let connection = format!(
            "{}:{}",
            parsed
                .host_str()
                .ok_or_else(|| DbError::ConnectionError(format!(
                    "Invalid DSN: Unparsable host {endpoint:?}"
                )))?,
            parsed.port().unwrap_or(DEFAULT_POSTGRES_PORT)
        );
        // Make sure the path is empty.
        if !parsed.path().is_empty() {
            return Err(DbError::ConnectionError(format!(
                "Invalid DSN: Table paths belong in AUTO*_DB_SETTINGS `tab: {endpoint:?}`"
            )));
        }
        debug!("📮 connection string {}", &connection);

        // Construct a new manager and put them in a pool for handling future requests.
        let manager = PostgresClientManager::new(
            &bt_settings,
            settings.dsn.clone(),
            connection.clone(),
            metrics.clone(),
        )?;
        let mut config = PoolConfig {
            // Prefer LIFO to allow the sweeper task to evict least frequently
            // used connections
            queue_mode: QueueMode::Lifo,
            ..Default::default()
        };
        if let Some(size) = bt_settings.database_pool_max_size {
            debug!("📮🏊 Setting pool max size {}", &size);
            config.max_size = size as usize;
        };
        config.timeouts = Timeouts {
            wait: bt_settings.database_pool_wait_timeout,
            create: bt_settings.database_pool_create_timeout,
            recycle: bt_settings.database_pool_recycle_timeout,
        };
        debug!("📮🏊 Timeouts: {:?}", &config.timeouts);

        let pool = deadpool::managed::Pool::builder(manager)
            .config(config)
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .map_err(|e| DbError::BTError(PostgresError::Config(e.to_string())))?;

        Ok(Self {
            pool,
            _metrics: metrics.clone(),
        })
    }

    /// Spawn a task to periodically evict idle connections
    pub fn spawn_sweeper(&self, interval: Duration) {
        let Some(max_idle) = self.pool.manager().settings.database_pool_max_idle else {
            return;
        };
        let pool = self.pool.clone();
        rt::spawn(async move {
            loop {
                sweeper(&pool, max_idle);
                rt::time::sleep(interval).await;
            }
        });
    }
}

fn sweeper(pool: &deadpool::managed::Pool<PostgresClientManager>, max_idle: Duration) {
    pool.retain(|_, metrics| metrics.last_used() < max_idle);
}

/// Postgres Pool Manager. This contains everything needed to create a new connection.
pub struct PostgresClientManager {
    settings: PostgresDbSettings,
    dsn: Option<String>,
    connection: String,
    metrics: Arc<StatsdClient>,
}

impl PostgresClientManager {
    fn new(
        settings: &PostgresDbSettings,
        dsn: Option<String>,
        connection: String,
        metrics: Arc<StatsdClient>,
    ) -> Result<Self, PostgresError> {
        Ok(Self {
            settings: settings.clone(),
            dsn,
            connection,
            metrics,
        })
    }
}

impl fmt::Debug for PostgresClientManager {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("deadpool::PostgresClientManager")
            .field("settings", &self.settings.clone())
            .finish()
    }
}

impl Manager for PostgresClientManager {
    type Error = PostgresError;
    type Type = PostgresDb;

    /// Create a new Postgres Client with it's own channel.
    /// `PostgresClient` is the most atomic we can go.
    async fn create(&self) -> Result<PostgresDb, Self::Error> {
        debug!("📮🏊 Create a new pool entry.");
        let entry = PostgresDb::new(
            self.get_channel()?,
            &self.settings.health_metadata()?,
            &self.settings.table_name,
        );
        debug!("📮🏊 Postgres connection acquired");
        Ok(entry)
    }

    /// Recycle if the connection has outlived it's lifespan.
    async fn recycle(
        &self,
        client: &mut Self::Type,
        metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Self::Error> {
        if let Some(timeout) = self.settings.database_pool_connection_ttl {
            if Instant::now() - metrics.created > timeout {
                debug!("📮🏊 Recycle requested (old).");
                return Err(RecycleError::message("Connection too old"));
            }
        }
        if let Some(timeout) = self.settings.database_pool_max_idle {
            if let Some(recycled) = metrics.recycled {
                if Instant::now() - recycled > timeout {
                    debug!("📮🏊 Recycle requested (idle).");
                    return Err(RecycleError::message("Connection too idle"));
                }
            }
        }

        if !client
            .health_check(&self.metrics.clone(), &self.settings.app_profile_id)
            .await
            .inspect_err(|e| debug!("📮🏊 Recycle requested (health). {:?}", e))?
        {
            debug!("📮🏊 Health check failed");
            return Err(RecycleError::message("Health check failed"));
        }

        Ok(())
    }
}

impl PostgresClientManager {
    /// Get a new Channel, based on the application settings.
    pub fn get_channel(&self) -> Result<Channel, PostgresError> {
        Ok(Self::create_channel(self.dsn.clone())?.connect(self.connection.as_str()))
    }

    /// Channels are GRPCIO constructs that contain the actual command data paths.
    /// Channels seem to be fairly light weight.
    pub fn create_channel(dsn: Option<String>) -> Result<ChannelBuilder, PostgresError> {
        debug!("📮🏊 Creating new channel...");
        let mut chan = ChannelBuilder::new(Arc::new(EnvBuilder::new().build()))
            .max_send_message_len(MAX_MESSAGE_LEN)
            .max_receive_message_len(MAX_MESSAGE_LEN);
        Ok(chan)
    }
}
