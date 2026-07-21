use std::{
    fmt,
    sync::Arc,
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
// Following defaults from Google's bigtable client grpc library
// https://github.com/googleapis/google-cloud-cpp/blob/f5f12f3cc5ee1293deab4c8e3c0d918bfa8c3b5a/google/cloud/bigtable/internal/defaults.cc#L63-L68
const DEFAULT_H2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_H2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);
// Mirroring H2_KEEPALIVE_INTERVAL
const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(30);

/// The pool of BigTable Clients.
/// Note: BigTable uses HTTP/2 as the backbone, so the only really important bit
/// that we have control over is the "channel". For now, we're using the ClientManager to
/// create new Bigtable clients, which have channels associated with them.
#[derive(Clone)]
pub struct BigTablePool {
    /// Pool of db connections
    pub pool: deadpool::managed::Pool<BigtableClientManager>,
    _metrics: Arc<StatsdClient>,
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
    ) -> Result<deadpool::managed::Object<BigtableClientManager>, BigTableError> {
        let obj = self.pool.get().await.map_err(|e| match e {
            PoolError::Timeout(tt) => BigTableError::PoolTimeout(tt),
            PoolError::Backend(e) => e,
            e => BigTableError::Pool(Box::new(e)),
        })?;
        debug!("🉑 Got db from pool");
        Ok(obj)
    }

    /// Creates a new pool of BigTable db connections.
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

        // Construct a new manager and put them in a pool for handling future requests.
        let manager =
            BigtableClientManager::new(&bt_settings, settings.dsn.clone(), connection.clone())?;
        let mut config = PoolConfig {
            // Prefer LIFO to allow the sweeper task to evict least frequently
            // used connections
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

fn sweeper(pool: &deadpool::managed::Pool<BigtableClientManager>, max_idle: Duration) {
    pool.retain(|_, metrics| metrics.last_used() < max_idle);
}

/// BigTable Pool Manager. This contains everything needed to create a new connection.
pub struct BigtableClientManager {
    settings: BigTableDbSettings,
    dsn: Option<String>,
    connection: String,
    /// Lazily initialized Application Default Credentials (ADC) token
    /// provider, shared across all pooled connections (it caches and
    /// refreshes tokens internally). `None` until first used; never
    /// initialized when running against the emulator.
    auth_provider: OnceCell<Arc<dyn TokenProvider>>,
}

impl BigtableClientManager {
    fn new(
        settings: &BigTableDbSettings,
        dsn: Option<String>,
        connection: String,
    ) -> Result<Self, BigTableError> {
        Ok(Self {
            settings: settings.clone(),
            dsn,
            connection,
            auth_provider: OnceCell::new(),
        })
    }

    /// Are we running against a local Bigtable emulator?
    fn is_emulator(&self) -> bool {
        self.dsn
            .as_ref()
            .map(|v| v.contains("localhost"))
            .unwrap_or(false)
            || std::env::var("BIGTABLE_EMULATOR_HOST").is_ok()
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

    /// Create a new Bigtable Client with it's own channel.
    /// `BigtableClient` is the most atomic we can go.
    async fn create(&self) -> Result<BigtableDb, Self::Error> {
        debug!("🏊 Create a new pool entry.");
        let entry = BigtableDb::new(
            self.get_channel()?,
            self.token_provider().await?,
            &self.settings.health_metadata()?,
            &self.settings.table_name,
        );
        debug!("🏊 Bigtable connection acquired");
        Ok(entry)
    }

    /// Recycle if the connection has outlived it's lifespan.
    async fn recycle(
        &self,
        _client: &mut Self::Type,
        metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Self::Error> {
        if let Some(timeout) = self.settings.database_pool_connection_ttl
            && Instant::now() - metrics.created > timeout
        {
            debug!("🏊 Recycle requested (old).");
            return Err(RecycleError::message("Connection too old"));
        }
        if let Some(timeout) = self.settings.database_pool_max_idle
            && let Some(recycled) = metrics.recycled
            && Instant::now() - recycled > timeout
        {
            debug!("🏊 Recycle requested (idle).");
            return Err(RecycleError::message("Connection too idle"));
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
    /// Get a new Channel, based on the application settings.
    pub fn get_channel(&self) -> Result<Channel, BigTableError> {
        Self::create_channel(
            &self.connection,
            self.is_emulator(),
            self.settings.database_pool_create_timeout,
        )
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
            .map_err(BigTableError::Connect)?
            // HTTP/2 keepalive pings detect a connection that dies *while an RPC
            // is in flight* (a stream is open) within roughly
            // interval + timeout, rather than riding the TCP retransmission
            // timeout. We intentionally do NOT set `keep_alive_while_idle(true)`:
            // these connections are pooled and mostly idle, and pinging while
            // idle (which hyper does not self-throttle, unlike grpcio's C-core)
            // risks a GOAWAY(ENHANCE_YOUR_CALM) from the GFE for pinging without
            // data more often than it permits. Idle-connection liveness is
            // instead handled by `tcp_keepalive` below plus retrying the
            // resulting transport error on the (idempotent) read path.
            .http2_keep_alive_interval(DEFAULT_H2_KEEPALIVE_INTERVAL)
            // If a ping isn't ACKed within this window, drop the connection.
            .keep_alive_timeout(DEFAULT_H2_KEEPALIVE_TIMEOUT)
            // OS-level keepalive probes on the socket itself. Unlike the HTTP/2
            // pings above, these fire on idle connections too, so a GFE-reaped
            // idle connection can be torn down before a request is handed it
            // (=> Os { code: 32, BrokenPipe }).
            .tcp_keepalive(Some(DEFAULT_TCP_KEEPALIVE));
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
