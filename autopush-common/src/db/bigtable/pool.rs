use std::{
    fmt,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};

use actix_web::rt;
use cadence::StatsdClient;
use deadpool::managed::{Manager, PoolConfig, PoolError, QueueMode, RecycleError, Timeouts};
use gcp_auth::TokenProvider;
use tokio::sync::OnceCell;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::db::DbSettings;
use crate::db::bigtable::{BigTableDbSettings, BigTableError, bigtable_client::BigtableDb};
use crate::db::error::{DbError, DbResult};

const DEFAULT_GRPC_PORT: u16 = 443;
const DEFAULT_GRPC_CHANNEL_COUNT: usize = 2;
/// Bigtable's documented maximum concurrent streams per gRPC connection.
const MAX_CONCURRENT_STREAMS_PER_CHANNEL: usize = 100;

/// Pool of Bigtable client handles used to limit application-level concurrency.
///
/// A deadpool entry is not a physical connection. Entries share a small,
/// bounded set of tonic channels, and each channel multiplexes concurrent RPCs
/// over HTTP/2.
#[derive(Clone)]
pub struct BigTablePool {
    /// Pool of logical operation handles.
    pub pool: deadpool::managed::Pool<BigtableClientManager>,
    _metrics: Arc<StatsdClient>,
}

impl fmt::Debug for BigTablePool {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("BigTablePool").finish()
    }
}

/// Several convenience functions for using the pool.
impl BigTablePool {
    /// Get a new managed object from the pool.
    pub async fn get(
        &self,
    ) -> Result<deadpool::managed::Object<BigtableClientManager>, BigTableError> {
        let mut obj = self.pool.get().await.map_err(|e| match e {
            PoolError::Timeout(tt) => BigTableError::PoolTimeout(tt),
            PoolError::Backend(e) => e,
            e => BigTableError::Pool(Box::new(e)),
        })?;
        // Select a channel per operation rather than permanently pinning a
        // frequently reused LIFO entry to one channel.
        obj.set_channel(self.pool.manager().get_channel());
        debug!("🉑 Got db from pool");
        Ok(obj)
    }

    /// Creates a new pool of Bigtable operation handles and shared channels.
    pub fn new(settings: &DbSettings, metrics: &Arc<StatsdClient>) -> DbResult<Self> {
        let Some(endpoint) = &settings.dsn else {
            return Err(DbError::ConnectionError(
                "No DSN specified in settings".to_owned(),
            ));
        };
        let bt_settings = BigTableDbSettings::try_from(settings.db_settings.as_str())?;
        debug!("🉑 DSN: {}", &endpoint);
        // Url::parsed() doesn't know how to handle `grpc:` schema, so it returns "null".
        let parsed = url::Url::parse(endpoint)
            .map_err(|e| DbError::ConnectionError(format!("Invalid DSN: {endpoint:?} : {e:?}")))?;
        let connection = format!(
            "{}:{}",
            parsed
                .host_str()
                .ok_or_else(|| DbError::ConnectionError(format!(
                    "Invalid DSN: Unparsable host {endpoint:?}"
                )))?,
            parsed.port().unwrap_or(DEFAULT_GRPC_PORT)
        );
        // Make sure the path is empty.
        if !parsed.path().is_empty() {
            return Err(DbError::ConnectionError(format!(
                "Invalid DSN: Table paths belong in AUTO*_DB_SETTINGS `tab: {endpoint:?}`"
            )));
        }
        debug!("🉑 connection string {}", &connection);

        let mut config = PoolConfig {
            // Prefer LIFO to allow the sweeper task to evict least frequently
            // used handles
            queue_mode: QueueMode::Lifo,
            ..Default::default()
        };
        if let Some(size) = bt_settings.database_pool_max_size {
            debug!("🏊 Setting pool max size {}", &size);
            config.max_size = size as usize;
        };
        config.timeouts = Timeouts {
            wait: bt_settings.database_pool_wait_timeout,
            create: bt_settings.database_pool_create_timeout,
            recycle: bt_settings.database_pool_recycle_timeout,
        };
        debug!("🏊 Timeouts: {:?}", &config.timeouts);

        let channel_count = bt_settings
            .grpc_channel_count
            .map(|count| count as usize)
            .unwrap_or(DEFAULT_GRPC_CHANNEL_COUNT);
        info!(
            "🏊 Sharing {channel_count} tonic channels across {} Bigtable operation slots",
            config.max_size
        );
        let stream_capacity = channel_count.saturating_mul(MAX_CONCURRENT_STREAMS_PER_CHANNEL);
        if config.max_size > stream_capacity {
            warn!(
                "Configured Bigtable operation pool size {} exceeds the nominal HTTP/2 stream capacity of {stream_capacity} across {channel_count} channels; requests may queue inside tonic",
                config.max_size
            );
        }

        // Construct a manager whose lightweight client handles share the
        // bounded tonic channel set.
        let manager = BigtableClientManager::new(
            &bt_settings,
            settings.dsn.clone(),
            connection,
            channel_count,
        )?;

        let pool = deadpool::managed::Pool::builder(manager)
            .config(config)
            .runtime(deadpool::Runtime::Tokio1)
            .build()
            .map_err(|e| DbError::BTError(BigTableError::Config(e.to_string())))?;

        Ok(Self {
            pool,
            _metrics: metrics.clone(),
        })
    }

    /// Spawn a task to periodically evict idle client handles.
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

    /// Number of tonic channels shared by all logical pool entries.
    pub fn configured_channel_count(&self) -> usize {
        self.pool.manager().channels.len()
    }
}

fn sweeper(pool: &deadpool::managed::Pool<BigtableClientManager>, max_idle: Duration) {
    pool.retain(|_, metrics| metrics.last_used() < max_idle);
}

/// Bigtable pool manager. This owns the bounded shared channel set and creates
/// lightweight client handles for deadpool.
pub struct BigtableClientManager {
    settings: BigTableDbSettings,
    dsn: Option<String>,
    channels: Vec<Channel>,
    next_channel: AtomicUsize,
    /// Lazily initialized Application Default Credentials (ADC) token
    /// provider, shared across all pooled handles (it caches and
    /// refreshes tokens internally). `None` until first used; never
    /// initialized when running against the emulator.
    auth_provider: OnceCell<Arc<dyn TokenProvider>>,
}

impl BigtableClientManager {
    fn new(
        settings: &BigTableDbSettings,
        dsn: Option<String>,
        connection: String,
        channel_count: usize,
    ) -> Result<Self, BigTableError> {
        let is_emulator = Self::is_emulator_dsn(dsn.as_deref());
        let channels = (0..channel_count)
            .map(|_| {
                Self::create_channel(
                    &connection,
                    is_emulator,
                    settings.database_pool_create_timeout,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            settings: settings.clone(),
            dsn,
            channels,
            next_channel: AtomicUsize::new(0),
            auth_provider: OnceCell::new(),
        })
    }

    fn is_emulator_dsn(dsn: Option<&str>) -> bool {
        dsn.map(|value| value.contains("localhost"))
            .unwrap_or(false)
            || std::env::var("BIGTABLE_EMULATOR_HOST").is_ok()
    }

    /// Are we running against a local Bigtable emulator?
    fn is_emulator(&self) -> bool {
        Self::is_emulator_dsn(self.dsn.as_deref())
    }

    /// Return the shared ADC token provider, or `None` when running against
    /// the emulator (which requires no credentials).
    async fn token_provider(&self) -> Result<Option<Arc<dyn TokenProvider>>, BigTableError> {
        if self.is_emulator() {
            debug!("🉑 Using emulator");
            return Ok(None);
        }
        debug!("🉑 Using real");
        let provider = self
            .auth_provider
            .get_or_try_init(|| async { gcp_auth::provider().await.map_err(BigTableError::Auth) })
            .await?;
        Ok(Some(provider.clone()))
    }
}

impl fmt::Debug for BigtableClientManager {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("deadpool::BtClientManager")
            .field("settings", &self.settings.clone())
            .finish()
    }
}

impl Manager for BigtableClientManager {
    type Error = BigTableError;
    type Type = BigtableDb;

    /// Create a lightweight client handle sharing one of the bounded channels.
    async fn create(&self) -> Result<BigtableDb, Self::Error> {
        debug!("🏊 Create a new pool entry.");
        let entry = BigtableDb::new(
            self.get_channel(),
            self.token_provider().await?,
            &self.settings.health_metadata()?,
            &self.settings.table_name,
        );
        debug!("🏊 Bigtable client handle acquired");
        Ok(entry)
    }

    /// Recycle if the client handle has outlived its lifespan.
    async fn recycle(
        &self,
        _client: &mut Self::Type,
        metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Self::Error> {
        if let Some(timeout) = self.settings.database_pool_connection_ttl
            && Instant::now() - metrics.created > timeout
        {
            debug!("🏊 Recycle requested (old).");
            return Err(RecycleError::message("Client handle too old"));
        }
        if let Some(timeout) = self.settings.database_pool_max_idle
            && let Some(recycled) = metrics.recycled
            && Instant::now() - recycled > timeout
        {
            debug!("🏊 Recycle requested (idle).");
            return Err(RecycleError::message("Client handle too idle"));
        }

        // A tonic Channel reconnects its transport as needed. Do not issue a
        // Bigtable ReadRows RPC merely to check a pooled object back out: that
        // doubles healthy traffic and can amplify an outage through retries.
        // The application's explicit health endpoint still performs a real
        // Bigtable health check.
        Ok(())
    }
}

impl BigtableClientManager {
    /// Clone the next shared channel in round-robin order. Tonic channel clones
    /// use the same underlying HTTP/2 connection and are cheap to create.
    fn get_channel(&self) -> Channel {
        let index = self.next_channel.fetch_add(1, Ordering::Relaxed) % self.channels.len();
        self.channels[index].clone()
    }

    /// Channels are the tonic transport constructs that contain the actual
    /// HTTP/2 connection. Channels are fairly light weight.
    pub fn create_channel(
        connection: &str,
        is_emulator: bool,
        connect_timeout: Option<Duration>,
    ) -> Result<Channel, BigTableError> {
        debug!("🏊 Creating new channel...");
        // The emulator runs plain HTTP/2 without TLS or credentials.
        let scheme = if is_emulator { "http" } else { "https" };
        let mut endpoint = Endpoint::from_shared(format!("{scheme}://{connection}"))
            .map_err(BigTableError::Connect)?;
        if !is_emulator {
            endpoint = endpoint
                .tls_config(ClientTlsConfig::new().with_native_roots())
                .map_err(BigTableError::Connect)?;
        }
        if let Some(connect_timeout) = connect_timeout {
            endpoint = endpoint.connect_timeout(connect_timeout);
        }
        Ok(endpoint.connect_lazy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[actix_rt::test]
    async fn channel_creation_is_lazy() {
        // Port 9 is intentionally not expected to host a Bigtable emulator.
        // Constructing the channel must still succeed without touching the
        // network; the first RPC is responsible for establishing a connection.
        let channel = BigtableClientManager::create_channel(
            "127.0.0.1:9",
            true,
            Some(Duration::from_millis(10)),
        );

        assert!(channel.is_ok());
    }
}
