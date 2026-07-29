use std::{
    fmt,
    future::Future,
    sync::Arc,
    sync::LazyLock,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};

use cadence::StatsdClient;
use deadpool::managed::{Manager, PoolConfig, PoolError, QueueMode, Timeouts};
use gcp_auth::TokenProvider;
use hyper::rt::Executor;
use tokio::runtime::{Handle, Runtime};
use tokio::sync::OnceCell;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

use crate::db::DbSettings;
use crate::db::bigtable::{BigTableDbSettings, BigTableError, bigtable_client::BigtableDb};
use crate::db::error::{DbError, DbResult};

const DEFAULT_GRPC_PORT: u16 = 443;
// These HTTP/2 keepalive values follow the Google Cloud C++ Bigtable client.
// They are new transport settings for autopush; the previous grpcio channel
// builder did not configure keepalive explicitly.
// https://github.com/googleapis/google-cloud-cpp/blob/f5f12f3cc5ee1293deab4c8e3c0d918bfa8c3b5a/google/cloud/bigtable/internal/defaults.cc#L63-L68
const DEFAULT_H2_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_H2_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);
/// Default number of shared channels.
///
/// Sixteen keeps wire capacity (16 x 100 streams) above any plausible
/// admission ceiling, so requests wait for a deadpool permit, which is visible
/// in `database.ops.queued` and covered by the operation budget, and never
/// inside hyper's dispatch queue, which is unbounded, invisible, and burns the
/// per-attempt deadline while a request is parked. The sizing case is
/// autoconnect's deploy-time reconnect storm: every client of a replaced pod
/// re-runs hello at once, each hello is a point read plus scans that hold
/// their stream slots, and any Bigtable latency blip multiplies concurrency
/// (Little's law) toward the wire ceiling. Four channels (400 streams) sat
/// below the deployed admission ceiling, which our own startup warning calls
/// out, and a storm that crosses the wire ceiling burns deadlines and slows
/// the shared-path health check, so congestion reads as backend failure.
///
/// To replace this with a sized value, set `grpc_channel_count` to
/// `ceil(C / 25)` using peak `database.ops.inflight`. Two corrections are needed
/// before that number means anything: the metric carries no hostname tag, so
/// divide by the pod count to get a per-process figure, and one Bigtable
/// instance serves both autoconnect and autoendpoint, so tag by workload and
/// size each service from its own peak.
/// https://cloud.google.com/bigtable/docs/configure-connection-pools
const DEFAULT_GRPC_CHANNEL_COUNT: usize = 16;
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

/// A fixed set of lazily-connected channels, selected round-robin.
///
/// Slots are never replaced. A tonic `Channel` owns a `Reconnect` that re-dials
/// on the next `poll_ready` after a transport failure, so a channel recovers on
/// its own. Bigtable's middleware deletes a connection that has not seen a
/// request in five minutes.
///
/// A reap discovered before dispatch is invisible: `poll_ready` fails, the
/// channel re-dials, and the request goes out on the new connection. A reap that
/// lands while a request is in flight leaves that request's outcome ambiguous,
/// and what happens next depends on the operation:
///
/// - Reads and `MutateRow` are replayed on another slot. Explicit cell
///   timestamps make a replay byte-identical, so the cost is the documented
///   server-side cache miss and a latency blip.
/// - `CheckAndMutateRow` is not replayed, because its predicate would be
///   re-evaluated against state the first attempt may have created. `add_user`
///   would then report a completed registration as `DbError::Conditional`, a
///   wrong answer rather than a failure. These surface as an error instead, and
///   the push client retries the registration.
struct SharedChannels {
    channels: Vec<Channel>,
    next_channel: AtomicUsize,
}

impl SharedChannels {
    fn new(endpoint: &Endpoint, count: usize) -> Self {
        // Settings validation rejects zero, but keep the invariant local so
        // `next` cannot divide by zero.
        let count = count.max(1);
        Self {
            channels: (0..count).map(|_| endpoint.connect_lazy()).collect(),
            next_channel: AtomicUsize::new(0),
        }
    }

    fn len(&self) -> usize {
        self.channels.len()
    }

    fn next(&self) -> Channel {
        let index = self.next_channel.fetch_add(1, Ordering::Relaxed) % self.channels.len();
        self.channels[index].clone()
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
            // Entries are interchangeable auth handles, so LIFO simply keeps the
            // working set small.
            queue_mode: QueueMode::Lifo,
            ..Default::default()
        };
        if let Some(size) = bt_settings.database_pool_max_size {
            debug!("🏊 Setting pool max size {}", &size);
            config.max_size = size as usize;
        };
        // No recycle timeout: `Manager::recycle` returns immediately, so deadpool
        // has nothing to bound there.
        config.timeouts = Timeouts {
            wait: bt_settings.database_pool_wait_timeout,
            create: bt_settings.database_pool_create_timeout,
            ..Default::default()
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
        let channels = Arc::new(SharedChannels::new(&endpoint, channel_count));
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

    /// A pool entry is an auth handle, not a connection, so it has no lifespan
    /// to enforce and nothing to health check. Channel liveness belongs to the
    /// shared channels, where tonic's `Reconnect` handles it, and the
    /// application's health endpoint still performs a real Bigtable RPC.
    async fn recycle(
        &self,
        _client: &mut Self::Type,
        _metrics: &deadpool::managed::Metrics,
    ) -> deadpool::managed::RecycleResult<Self::Error> {
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
    use super::*;

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
        let channels = SharedChannels::new(&endpoint, 2);

        let _first = channels.next();
        assert_eq!(channels.next_channel.load(Ordering::Relaxed), 1);
        let _second = channels.next();
        assert_eq!(channels.next_channel.load(Ordering::Relaxed), 2);
        let _wrapped = channels.next();
        assert_eq!(channels.next_channel.load(Ordering::Relaxed), 3);
    }
}
