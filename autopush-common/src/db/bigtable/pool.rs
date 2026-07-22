use std::{
    fmt,
    future::Future,
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
    sync::{LazyLock, RwLock, Weak},
    time::{Duration, Instant},
};

use cadence::StatsdClient;
use deadpool::managed::{Manager, PoolConfig, PoolError, QueueMode, RecycleError, Timeouts};
use gcp_auth::TokenProvider;
use hyper::rt::Executor;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::OnceCell;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::db::DbSettings;
use crate::db::bigtable::{BigTableDbSettings, BigTableError, bigtable_client::BigtableDb};
use crate::db::error::{DbError, DbResult};
use crate::metric_name::MetricName;
use crate::metrics::StatsdClientExt;

const DEFAULT_GRPC_PORT: u16 = 443;
// These HTTP/2 keepalive values follow the Google Cloud C++ Bigtable client.
// They are new transport settings for autopush; the previous grpcio channel
// builder did not configure keepalive explicitly.
// https://github.com/googleapis/google-cloud-cpp/blob/f5f12f3cc5ee1293deab4c8e3c0d918bfa8c3b5a/google/cloud/bigtable/internal/defaults.cc#L63-L68
const DEFAULT_H2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_H2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_GRPC_CHANNEL_COUNT: usize = 2;
/// Bigtable's documented maximum concurrent streams per gRPC connection.
const MAX_CONCURRENT_STREAMS_PER_CHANNEL: usize = 100;

/// Tonic normally spawns its buffer and Hyper connection drivers onto the
/// runtime where an Endpoint is constructed. Autopush constructs the database
/// before Actix starts its worker runtimes, so using tonic's default executor
/// pins all Bigtable I/O to Actix's single-threaded main runtime. Keep transport
/// work on a small, explicit multithreaded runtime instead.
static BIGTABLE_TRANSPORT_RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("bigtable-transport")
        .enable_all()
        .build()
        .expect("failed to create the Bigtable transport runtime")
});

#[derive(Clone)]
struct BigtableExecutor {
    handle: Handle,
}

impl<F> Executor<F> for BigtableExecutor
where
    F: Future<Output = ()> + Send + 'static,
{
    fn execute(&self, future: F) {
        self.handle.spawn(future);
    }
}

struct ChannelSlot {
    channel: Channel,
    generation: u64,
}

struct SharedChannels {
    endpoint: Endpoint,
    channels: RwLock<Vec<ChannelSlot>>,
    next_channel: AtomicUsize,
    runtime: Handle,
    refresh_timeout: Duration,
}

impl SharedChannels {
    fn new(endpoint: Endpoint, count: usize, refresh_timeout: Duration) -> Self {
        let channels = (0..count)
            .map(|_| ChannelSlot {
                channel: endpoint.connect_lazy(),
                generation: 0,
            })
            .collect();
        Self {
            endpoint,
            channels: RwLock::new(channels),
            next_channel: AtomicUsize::new(0),
            runtime: BIGTABLE_TRANSPORT_RUNTIME.handle().clone(),
            refresh_timeout,
        }
    }

    fn len(&self) -> usize {
        self.channels
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    fn next(&self) -> Channel {
        let channels = self
            .channels
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let index = self.next_channel.fetch_add(1, Ordering::Relaxed) % channels.len();
        channels[index].channel.clone()
    }

    fn replace(&self, index: usize, channel: Channel) -> bool {
        let mut slots = self
            .channels
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(slot) = slots.get_mut(index) else {
            return false;
        };
        slot.channel = channel;
        slot.generation = slot.generation.saturating_add(1);
        true
    }

    #[cfg(test)]
    fn generation(&self, index: usize) -> u64 {
        self.channels
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())[index]
            .generation
    }

    fn spawn_refresher(
        channels: &Arc<Self>,
        min_period: Duration,
        max_period: Duration,
        metrics: Arc<StatsdClient>,
    ) {
        for index in 0..channels.len() {
            let weak = Arc::downgrade(channels);
            let metrics = metrics.clone();
            channels.runtime.spawn(async move {
                channel_refresh_loop(weak, index, min_period, max_period, metrics).await;
            });
        }
    }
}

async fn refresh_channel_once(
    channels: &SharedChannels,
    index: usize,
    metrics: &StatsdClient,
) -> bool {
    match tokio::time::timeout(channels.refresh_timeout, channels.endpoint.connect()).await {
        Ok(Ok(channel)) => {
            if channels.replace(index, channel) {
                let _ = metrics.incr(MetricName::DatabaseChannelRefreshSuccess);
                true
            } else {
                warn!("Bigtable channel {index} disappeared before refresh replacement");
                let _ = metrics.incr(MetricName::DatabaseChannelRefreshFailure);
                false
            }
        }
        Ok(Err(error)) => {
            warn!("Bigtable channel {index} refresh failed: {error}");
            let _ = metrics.incr(MetricName::DatabaseChannelRefreshFailure);
            false
        }
        Err(_) => {
            warn!(
                "Bigtable channel {index} refresh exceeded {:?}",
                channels.refresh_timeout
            );
            let _ = metrics.incr(MetricName::DatabaseChannelRefreshFailure);
            false
        }
    }
}

async fn channel_refresh_loop(
    channels: Weak<SharedChannels>,
    index: usize,
    min_period: Duration,
    max_period: Duration,
    metrics: Arc<StatsdClient>,
) {
    loop {
        let min_secs = min_period.as_secs();
        let max_secs = max_period.as_secs();
        let delay = Duration::from_secs(rand::random_range(min_secs..=max_secs));
        tokio::time::sleep(delay).await;

        let Some(channels) = channels.upgrade() else {
            return;
        };
        refresh_channel_once(&channels, index, &metrics).await;
    }
}

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
        let obj = self.pool.get().await.map_err(|e| match e {
            PoolError::Timeout(tt) => BigTableError::PoolTimeout(tt),
            PoolError::Backend(e) => e,
            e => BigTableError::Pool(Box::new(e)),
        })?;
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
        if !manager.is_emulator() {
            SharedChannels::spawn_refresher(
                &manager.channels,
                bt_settings.grpc_channel_refresh_min,
                bt_settings.grpc_channel_refresh_max,
                metrics.clone(),
            );
        }

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
        BIGTABLE_TRANSPORT_RUNTIME.spawn(async move {
            loop {
                sweeper(&pool, max_idle);
                tokio::time::sleep(interval).await;
            }
        });
    }

    /// Number of tonic channels shared by all logical pool entries.
    pub fn configured_channel_count(&self) -> usize {
        self.pool.manager().channels.len()
    }

    /// Select a shared channel for one RPC attempt. Retried operations call
    /// this again so a dead transport does not consume the entire retry budget.
    pub(super) fn next_channel(&self) -> Channel {
        self.pool.manager().channels.next()
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
    channels: Arc<SharedChannels>,
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
        let endpoint =
            Self::create_endpoint(&connection, is_emulator, settings.grpc_connect_timeout)?;
        // Endpoint::connect_timeout covers the TCP connector. The outer refresh
        // timeout also bounds TLS and guards the complete refresh operation.
        let refresh_timeout = settings.grpc_connect_timeout.saturating_mul(2);
        let channels = Arc::new(SharedChannels::new(
            endpoint,
            channel_count,
            refresh_timeout,
        ));
        Ok(Self {
            settings: settings.clone(),
            dsn,
            channels,
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
        let entry = BigtableDb::new(self.token_provider().await?);
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
    fn create_endpoint(
        connection: &str,
        is_emulator: bool,
        connect_timeout: Duration,
    ) -> Result<Endpoint, BigTableError> {
        debug!("🏊 Creating Bigtable endpoint...");
        // The emulator runs plain HTTP/2 without TLS or credentials.
        let scheme = if is_emulator { "http" } else { "https" };
        let mut endpoint = Endpoint::from_shared(format!("{scheme}://{connection}"))
            .map_err(BigTableError::Connect)?
            // Detect a dead connection while an RPC stream is active. These
            // are HTTP/2 PINGs, not TCP keepalive probes.
            .http2_keep_alive_interval(DEFAULT_H2_KEEPALIVE_INTERVAL)
            // If a ping isn't ACKed within this window, drop the connection.
            .keep_alive_timeout(DEFAULT_H2_KEEPALIVE_TIMEOUT)
            // Do not ping an idle channel. Bigtable intentionally reaps idle
            // connections, and excessive pings can trigger ENHANCE_YOUR_CALM.
            .keep_alive_while_idle(false)
            .connect_timeout(connect_timeout)
            .executor(BigtableExecutor {
                handle: BIGTABLE_TRANSPORT_RUNTIME.handle().clone(),
            });
        if !is_emulator {
            endpoint = endpoint
                .tls_config(
                    ClientTlsConfig::new()
                        .with_native_roots()
                        .timeout(connect_timeout),
                )
                .map_err(BigTableError::Connect)?;
        }
        Ok(endpoint)
    }

    /// Create one lazy channel for tests and callers that only need to verify
    /// endpoint construction.
    #[cfg(test)]
    pub fn create_channel(
        connection: &str,
        is_emulator: bool,
        connect_timeout: Duration,
    ) -> Result<Channel, BigTableError> {
        Ok(Self::create_endpoint(connection, is_emulator, connect_timeout)?.connect_lazy())
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;

    use http_body_util::Empty;
    use hyper::body::{Bytes, Incoming};
    use hyper::server::conn::http2;
    use hyper::service::service_fn;
    use hyper::{Request, Response};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use tokio::net::TcpListener;

    use super::*;

    fn test_metrics() -> StatsdClient {
        StatsdClient::builder("", cadence::NopMetricSink).build()
    }

    #[actix_rt::test]
    async fn channel_creation_is_lazy() {
        // Port 9 is intentionally not expected to host a Bigtable emulator.
        // Constructing the channel must still succeed without touching the
        // network; the first RPC is responsible for establishing a connection.
        let channel =
            BigtableClientManager::create_channel("127.0.0.1:9", true, Duration::from_millis(10));

        assert!(channel.is_ok());
    }

    #[test]
    fn channel_selection_advances_for_each_attempt() {
        let endpoint =
            BigtableClientManager::create_endpoint("127.0.0.1:9", true, Duration::from_millis(10))
                .unwrap();
        let channels = SharedChannels::new(endpoint, 2, Duration::from_millis(20));

        let _first = channels.next();
        assert_eq!(channels.next_channel.load(Ordering::Relaxed), 1);
        let _second = channels.next();
        assert_eq!(channels.next_channel.load(Ordering::Relaxed), 2);
        let _wrapped = channels.next();
        assert_eq!(channels.next_channel.load(Ordering::Relaxed), 3);
    }

    #[actix_rt::test]
    async fn failed_refresh_retains_the_existing_slot() {
        let endpoint =
            BigtableClientManager::create_endpoint("127.0.0.1:9", true, Duration::from_millis(10))
                .unwrap();
        let channels = SharedChannels::new(endpoint, 1, Duration::from_millis(20));

        assert!(!refresh_channel_once(&channels, 0, &test_metrics()).await);
        assert_eq!(channels.generation(0), 0);
    }

    #[actix_rt::test]
    async fn refresh_timeout_covers_a_stalled_tls_handshake() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let stalled_peer = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            futures::future::pending::<()>().await;
        });
        let endpoint = BigtableClientManager::create_endpoint(
            &address.to_string(),
            false,
            Duration::from_secs(1),
        )
        .unwrap();
        let channels = SharedChannels::new(endpoint, 1, Duration::from_millis(20));

        assert!(!refresh_channel_once(&channels, 0, &test_metrics()).await);
        assert_eq!(channels.generation(0), 0);
        stalled_peer.abort();
    }

    #[actix_rt::test]
    async fn successful_refresh_replaces_the_slot() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let service = service_fn(|_request: Request<Incoming>| async move {
                Ok::<_, Infallible>(Response::new(Empty::<Bytes>::new()))
            });
            http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(socket), service)
                .await
        });
        let endpoint = BigtableClientManager::create_endpoint(
            &address.to_string(),
            true,
            Duration::from_secs(1),
        )
        .unwrap();
        let channels = SharedChannels::new(endpoint, 1, Duration::from_secs(1));

        assert!(refresh_channel_once(&channels, 0, &test_metrics()).await);
        assert_eq!(channels.generation(0), 1);

        drop(channels);
        let _result = tokio::time::timeout(Duration::from_secs(1), server)
            .await
            .expect("refresh test server did not shut down")
            .unwrap();
    }
}
