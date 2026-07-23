use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error as StdError;
use std::fmt;
use std::fmt::Display;
use std::future::Future;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use again::RetryPolicy;
use async_trait::async_trait;
use cadence::StatsdClient;
#[cfg(feature = "reliable_report")]
use chrono::TimeDelta;
use gcp_auth::TokenProvider;
use googleapis_tonic_google_bigtable_v2::google::bigtable::v2 as bigtable;
use googleapis_tonic_google_bigtable_v2::google::bigtable::v2::bigtable_client::BigtableClient;
use serde_json::{from_str, json};
use tonic::metadata::{AsciiMetadataValue, MetadataMap};
use tonic::transport::Channel;
use tonic::{Code, Request, Status};
use uuid::Uuid;

use crate::MAX_ROUTER_TTL_SECS;
use crate::db::{
    DbSettings, Notification, USER_RECORD_VERSION, User,
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    models::RangeKey,
};
use crate::metric_name::MetricName;
use crate::metrics::StatsdClientExt;

pub use self::metadata::MetadataBuilder;
use self::row::{Row, RowCells};
use super::BigTableDbSettings;
use super::pool::BigTablePool;

pub mod cell;
pub mod error;
pub(crate) mod merge;
pub mod metadata;
pub mod row;

// these are normally Vec<u8>
pub type RowKey = String;

// These are more for code clarity than functional types.
// Rust will happily swap between the two in any case.
// See [super::row::Row] for discussion about how these
// are overloaded in order to simplify fetching data.
pub type Qualifier = String;
pub type FamilyId = String;

const ROUTER_FAMILY: &str = "router";
const MESSAGE_FAMILY: &str = "message"; // The default family for messages
const MESSAGE_TOPIC_FAMILY: &str = "message_topic";
#[cfg(feature = "reliable_report")]
const RELIABLE_LOG_FAMILY: &str = "reliability";
#[cfg(feature = "reliable_report")]
/// The maximum TTL for reliability logging (60 days).
/// /// In most use cases, converted to seconds through .num_seconds().
pub const RELIABLE_LOG_TTL: TimeDelta = TimeDelta::days(60);

/// Default number of retries after the initial Bigtable RPC attempt.
///
/// So a connectivity failure cannot fan out into a
/// retry storm, we try to keep this number small.
/// Can be overridden through `db_settings.retry_count`; health checks use the
/// same configured value as other point reads.
pub(crate) const RETRY_COUNT: usize = 2;

/// Maximum gRPC message size (256MB), matching the prior grpcio configuration.
const MAX_MESSAGE_LEN: usize = 1 << 28;

/// OAuth2 scopes requested for the Bigtable data API.
const BIGTABLE_DATA_SCOPES: &[&str] = &["https://www.googleapis.com/auth/bigtable.data"];

/// Simple circuit breaker to prevent retry storms during BigTable outages.
///
/// After `failure_threshold` consecutive failures, the circuit opens and
/// requests fail fast for `cooldown_secs` seconds before allowing a retry.
#[derive(Debug)]
pub struct CircuitBreaker {
    consecutive_failures: AtomicU32,
    opened_at_epoch_secs: AtomicU64,
    failure_threshold: u32,
    cooldown_secs: u64,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, cooldown_secs: u64) -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            opened_at_epoch_secs: AtomicU64::new(0),
            failure_threshold,
            cooldown_secs,
        }
    }

    /// Check if the circuit is allowing requests through.
    /// Returns true if the request should proceed, false if it should fail fast.
    pub fn allow_request(&self) -> bool {
        let failures = self.consecutive_failures.load(Ordering::Relaxed);
        if failures < self.failure_threshold {
            return true;
        }
        // Circuit is open — check if cooldown has elapsed
        let opened_at = self.opened_at_epoch_secs.load(Ordering::Relaxed);
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if now.saturating_sub(opened_at) >= self.cooldown_secs {
            // Allow a single probe request (half-open state)
            true
        } else {
            false
        }
    }

    /// Record a successful operation, resetting the circuit breaker.
    pub fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    /// Record a failed operation.
    pub fn record_failure(&self) {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        if prev + 1 >= self.failure_threshold {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            self.opened_at_epoch_secs.store(now, Ordering::Relaxed);
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        // Open after 5 consecutive failures, cooldown for 30 seconds
        Self::new(5, 30)
    }
}

/// Semi convenience wrapper to ensure that the UAID is formatted and displayed consistently.
// TODO:Should we create something similar for ChannelID?
struct Uaid(Uuid);

impl Display for Uaid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0.as_simple())
    }
}

impl From<Uaid> for String {
    fn from(uaid: Uaid) -> String {
        uaid.0.as_simple().to_string()
    }
}

#[derive(Clone)]
/// Bigtable-backed implementation of the application's database interface.
pub struct BigTableClientImpl {
    pub(crate) settings: BigTableDbSettings,
    /// Metrics client
    metrics: Arc<StatsdClient>,
    /// Logical-operation pool and shared tonic channel set.
    pool: BigTablePool,
    metadata: MetadataMap,
    /// Circuit breaker to prevent retry storms during BigTable outages
    circuit_breaker: Arc<CircuitBreaker>,
}

/// Return a a RowFilter matching the GC policy of the router Column Family
fn router_gc_policy_filter() -> bigtable::RowFilter {
    bigtable::RowFilter {
        filter: Some(bigtable::row_filter::Filter::CellsPerColumnLimitFilter(1)),
    }
}

/// Return a chain of RowFilters matching the GC policy of the message Column
/// Families
fn message_gc_policy_filter() -> Result<Vec<bigtable::RowFilter>, error::BigTableError> {
    let bt_now: i64 = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(error::BigTableError::WriteTime)?
        .as_millis() as i64;
    let timestamp_filter = bigtable::RowFilter {
        filter: Some(bigtable::row_filter::Filter::TimestampRangeFilter(
            bigtable::TimestampRange {
                start_timestamp_micros: bt_now * 1000,
                end_timestamp_micros: 0,
            },
        )),
    };

    Ok(vec![router_gc_policy_filter(), timestamp_filter])
}

/// Return a Column family regex RowFilter
fn family_filter(regex: String) -> bigtable::RowFilter {
    bigtable::RowFilter {
        filter: Some(bigtable::row_filter::Filter::FamilyNameRegexFilter(regex)),
    }
}

/// Escape bytes for RE values
///
/// Based off google-re2/perl's quotemeta function
fn escape_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut vec = Vec::with_capacity(bytes.len() * 2);
    for &b in bytes {
        if !b.is_ascii_alphanumeric() && b != b'_' && (b & 128) == 0 {
            if b == b'\0' {
                // Special handling for null: Note that this special handling
                // is not strictly required for RE2, but this quoting is
                // required for other regexp libraries such as PCRE.
                // Can't use "\\0" since the next character might be a digit.
                vec.extend("\\x00".as_bytes());
                continue;
            }
            vec.push(b'\\');
        }
        vec.push(b);
    }
    vec
}

/// Return a chain of RowFilters limiting to a match of the specified
/// `version`'s column value
fn version_filter(version: &Uuid) -> Vec<bigtable::RowFilter> {
    let cq_filter = bigtable::RowFilter {
        filter: Some(bigtable::row_filter::Filter::ColumnQualifierRegexFilter(
            "^version$".as_bytes().to_vec(),
        )),
    };
    let value_filter = bigtable::RowFilter {
        filter: Some(bigtable::row_filter::Filter::ValueRegexFilter(
            escape_bytes(version.as_bytes()),
        )),
    };

    vec![
        family_filter(format!("^{ROUTER_FAMILY}$")),
        cq_filter,
        value_filter,
    ]
}

/// Return a newly generated `version` column `Cell`
fn new_version_cell(timestamp: SystemTime) -> cell::Cell {
    cell::Cell {
        qualifier: "version".to_owned(),
        value: Uuid::new_v4().into(),
        timestamp,
        ..Default::default()
    }
}

/// Return a RowFilter chain from multiple RowFilters
fn filter_chain(filters: Vec<bigtable::RowFilter>) -> bigtable::RowFilter {
    bigtable::RowFilter {
        filter: Some(bigtable::row_filter::Filter::Chain(
            bigtable::row_filter::Chain { filters },
        )),
    }
}

/// Return a ReadRowsRequest against table for a given row key
fn read_row_request(
    table_name: &str,
    app_profile_id: &str,
    row_key: &str,
) -> bigtable::ReadRowsRequest {
    bigtable::ReadRowsRequest {
        table_name: table_name.to_owned(),
        app_profile_id: app_profile_id.to_owned(),
        rows: Some(bigtable::RowSet {
            row_keys: vec![row_key.as_bytes().to_vec()],
            row_ranges: Vec::new(),
        }),
        ..Default::default()
    }
}

fn to_u64(value: Vec<u8>, name: &str) -> Result<u64, DbError> {
    let v: [u8; 8] = value
        .try_into()
        .map_err(|_| DbError::DeserializeU64(name.to_owned()))?;
    Ok(u64::from_be_bytes(v))
}

fn to_string(value: Vec<u8>, name: &str) -> Result<String, DbError> {
    String::from_utf8(value).map_err(|e| {
        debug!("🉑 cannot read string {}: {:?}", name, e);
        DbError::DeserializeString(name.to_owned())
    })
}

/// Parse the "set" (see [DbClient::add_channels]) of channel ids in a bigtable Row.
///
/// Cells should solely contain the set of channels otherwise an Error is returned.
fn channels_from_cells(cells: &RowCells) -> DbResult<HashSet<Uuid>> {
    let mut result = HashSet::new();
    for cells in cells.values() {
        let Some(cell) = cells.last() else {
            continue;
        };
        let Some((_, chid)) = cell.qualifier.split_once("chid:") else {
            return Err(DbError::Integrity(
                "get_channels expected: chid:<chid>".to_owned(),
                None,
            ));
        };
        result.insert(Uuid::from_str(chid).map_err(|e| DbError::General(e.to_string()))?);
    }
    Ok(result)
}

/// Convert the [HashSet] of channel ids to cell entries for a bigtable Row
fn channels_to_cells(channels: Cow<HashSet<Uuid>>, expiry: SystemTime) -> Vec<cell::Cell> {
    let channels = channels.into_owned();
    let mut cells = Vec::with_capacity(channels.len().min(100_000));
    for (i, channel_id) in channels.into_iter().enumerate() {
        // There is a limit of 100,000 mutations per batch for bigtable.
        // https://cloud.google.com/bigtable/quotas
        // If you have 100,000 channels, you have too many.
        if i >= 100_000 {
            break;
        }
        cells.push(cell::Cell {
            qualifier: format!("chid:{}", channel_id.as_hyphenated()),
            timestamp: expiry,
            ..Default::default()
        });
    }
    cells
}

pub fn retry_policy(max: usize) -> RetryPolicy {
    RetryPolicy::default()
        .with_max_retries(max)
        .with_jitter(true)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RetryKind {
    Read,
    IdempotentWrite,
    ConditionalWrite,
}

#[derive(Clone, Copy, Debug)]
enum RpcClass {
    Point,
    Scan,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BreakerPolicy {
    Track,
    Ignore,
}

#[derive(Clone, Copy, Debug)]
struct RpcPolicy {
    attempt_timeout: Duration,
    total_timeout: Duration,
    retry_kind: RetryKind,
    breaker: BreakerPolicy,
}

async fn rpc_attempt_until<T>(
    deadline: tokio::time::Instant,
    future: impl Future<Output = Result<T, error::BigTableError>>,
) -> Result<T, error::BigTableError> {
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| error::BigTableError::AttemptTimeout)?
}

async fn operation_budget_until<T>(
    deadline: tokio::time::Instant,
    future: impl Future<Output = Result<T, error::BigTableError>>,
) -> Result<T, error::BigTableError> {
    tokio::time::timeout_at(deadline, future)
        .await
        .map_err(|_| error::BigTableError::OperationTimeout)?
}

fn retryable_pre_send_err(status: &Status) -> bool {
    (status.code() == Code::Unknown
        && status
            .message()
            .to_ascii_lowercase()
            .starts_with("service was not ready:"))
        || has_connect_source(status)
}

fn retryable_transient_err(status: &Status) -> bool {
    if retryable_pre_send_err(status) {
        return true;
    }
    match status.code() {
        Code::Internal => {
            let message = status.message().to_ascii_lowercase();
            [
                "rst_stream",
                "rst stream",
                "received unexpected eos on data frame from server",
            ]
            .iter()
            .any(|fragment| message.contains(fragment))
        }
        Code::Unavailable | Code::DeadlineExceeded => true,
        _ => false,
    }
}

/// Return whether a failed read is safe to retry.
///
/// In addition to the statuses that are retryable for every operation, tonic
/// can surface HTTP/2 connection retirement as either:
///
/// - `Internal("h2 protocol error: http2 error")` for a remote GOAWAY, or
/// - `Cancelled("operation was canceled")` when hyper closes the connection, or
/// - `Unknown("transport error")` when a connection-level failure (e.g. a
///   GFE-reaped idle connection surfacing as a broken pipe) is written to.
///
/// Reads are idempotent, so replaying these bounded attempts is safe. Autopush
/// `MutateRow` requests are also replay-safe: set-cell retries reuse the same
/// explicit timestamp, delete mutations are idempotent, and every attempt
/// clones the same request. Conditional `CheckAndMutateRow` operations use the
/// narrower pre-send-only policy because their returned predicate result can
/// change after an applied-but-lost attempt.
fn retryable_read_err(status: &Status) -> bool {
    if retryable_transient_err(status) {
        return true;
    }
    // Tonic 0.14.6 uses both Internal (for some header/GOAWAY paths) and
    // Unknown (for a connection lost while streaming a body) for this wrapper
    // message. The exact Hyper/H2 suffix is not a stable API.
    if matches!(status.code(), Code::Internal | Code::Unknown)
        && status
            .message()
            .to_ascii_lowercase()
            .starts_with("h2 protocol error:")
    {
        return true;
    }
    match status.code() {
        Code::Cancelled => status
            .message()
            .eq_ignore_ascii_case("operation was canceled"),
        // Tonic retains the transport and OS errors in the source chain. Use
        // their types rather than matching an unstable display string.
        Code::Unknown => has_transport_source(status),
        _ => false,
    }
}

fn has_transport_source(status: &Status) -> bool {
    let mut source = StdError::source(status);
    while let Some(error) = source {
        if error.downcast_ref::<tonic::transport::Error>().is_some()
            || error.downcast_ref::<std::io::Error>().is_some()
        {
            return true;
        }
        source = error.source();
    }
    false
}

fn has_connect_source(status: &Status) -> bool {
    let mut source = StdError::source(status);
    while let Some(error) = source {
        if error.downcast_ref::<tonic::ConnectError>().is_some() {
            return true;
        }
        source = error.source();
    }
    false
}

fn counts_toward_breaker(error: &error::BigTableError, operation_timed_out_in_rpc: bool) -> bool {
    matches!(
        error,
        error::BigTableError::InvalidRowResponse(_)
            | error::BigTableError::InvalidChunk(_)
            | error::BigTableError::Read(_)
            | error::BigTableError::Write(_)
            | error::BigTableError::AttemptTimeout
            | error::BigTableError::Status(_, _)
    ) || matches!(error, error::BigTableError::OperationTimeout) && operation_timed_out_in_rpc
}

#[cfg(test)]
mod retry_tests {
    use std::convert::Infallible;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    use futures::StreamExt;
    use http_body_util::StreamBody;
    use hyper::body::{Bytes, Frame, Incoming};
    use hyper::server::conn::http2;
    use hyper::service::service_fn;
    use hyper::{Request as HyperRequest, Response as HyperResponse};
    use hyper_util::rt::{TokioExecutor, TokioIo};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;

    use super::*;
    #[derive(Debug, thiserror::Error)]
    #[error("transport error")]
    struct TestTransportError {
        #[source]
        source: std::io::Error,
    }

    fn broken_pipe_status() -> Status {
        Status::from_error(Box::new(TestTransportError {
            source: std::io::Error::from(std::io::ErrorKind::BrokenPipe),
        }))
    }

    #[test]
    fn retries_tonic_pre_send_readiness_errors() {
        let status = Status::unknown("Service was not ready: transport error");
        assert!(retryable_pre_send_err(&status));
    }

    #[test]
    fn retries_source_backed_connect_errors_before_conditional_writes() {
        let connect = tonic::ConnectError(Box::new(std::io::Error::from(
            std::io::ErrorKind::ConnectionRefused,
        )));
        let status = Status::from_error(Box::new(connect));

        assert_eq!(status.code(), Code::Unavailable);
        assert!(retryable_pre_send_err(&status));
    }

    #[test]
    fn does_not_retry_arbitrary_unknown_errors() {
        let status = Status::unknown("operation outcome is unknown");
        assert!(!retryable_pre_send_err(&status));
    }

    #[test]
    fn retries_transient_bigtable_statuses() {
        assert!(retryable_transient_err(&Status::unavailable(
            "No zones were available"
        )));
        assert!(retryable_transient_err(&Status::deadline_exceeded(
            "deadline exceeded"
        )));
        assert!(retryable_transient_err(&Status::internal(
            "stream terminated by RST_STREAM before headers"
        )));
    }

    #[test]
    fn retries_observed_tonic_transport_failures_for_reads() {
        let goaway = Status::internal("h2 protocol error: http2 error");
        assert!(retryable_read_err(&goaway));
        assert!(classify_retry(RetryKind::Read, &error::BigTableError::Read(goaway)).is_some());

        let connection_closed = Status::cancelled("operation was canceled");
        assert!(retryable_read_err(&connection_closed));
        assert!(
            classify_retry(
                RetryKind::Read,
                &error::BigTableError::Read(connection_closed)
            )
            .is_some()
        );

        // A GFE-reaped idle connection written to surfaces as
        // `Unknown("transport error")` (the "broken pipe" io error is in the
        // source, not the status message).
        let transport_error = broken_pipe_status();
        assert_eq!(transport_error.code(), Code::Unknown);
        assert_eq!(transport_error.message(), "transport error");
        assert!(retryable_read_err(&transport_error));
        assert!(
            classify_retry(
                RetryKind::Read,
                &error::BigTableError::Read(transport_error)
            )
            .is_some()
        );
    }

    #[test]
    fn does_not_retry_non_transport_unknown_reads() {
        // `Unknown` is a catch-all that also covers server-returned application
        // errors; require a transport or IO error in the source chain.
        assert!(!retryable_read_err(&Status::unknown(
            "some application error"
        )));
    }

    #[test]
    fn retries_idempotent_but_not_conditional_writes() {
        let goaway =
            error::BigTableError::Write(Status::internal("h2 protocol error: http2 error"));
        assert!(classify_retry(RetryKind::IdempotentWrite, &goaway).is_some());
        let conditional_goaway =
            error::BigTableError::Write(Status::internal("h2 protocol error: http2 error"));
        assert!(classify_retry(RetryKind::ConditionalWrite, &conditional_goaway).is_none());

        let connection_closed =
            error::BigTableError::Write(Status::cancelled("operation was canceled"));
        assert!(classify_retry(RetryKind::IdempotentWrite, &connection_closed).is_some());

        let transport_error = error::BigTableError::Write(broken_pipe_status());
        assert!(classify_retry(RetryKind::IdempotentWrite, &transport_error).is_some());

        let unavailable = error::BigTableError::Write(Status::unavailable("try again"));
        assert!(classify_retry(RetryKind::IdempotentWrite, &unavailable).is_some());
        assert!(classify_retry(RetryKind::ConditionalWrite, &unavailable).is_none());

        let attempt_timeout = error::BigTableError::AttemptTimeout;
        assert!(classify_retry(RetryKind::Read, &attempt_timeout).is_some());
        assert!(classify_retry(RetryKind::ConditionalWrite, &attempt_timeout).is_none());
    }

    #[actix_rt::test]
    async fn attempt_and_operation_timeouts_are_enforced() {
        let attempt = rpc_attempt_until::<()>(
            tokio::time::Instant::now() + Duration::from_millis(1),
            async { futures::future::pending().await },
        )
        .await;
        assert!(matches!(attempt, Err(error::BigTableError::AttemptTimeout)));

        let operation = operation_budget_until::<()>(
            tokio::time::Instant::now() + Duration::from_millis(1),
            async { futures::future::pending().await },
        )
        .await;
        assert!(matches!(
            operation,
            Err(error::BigTableError::OperationTimeout)
        ));
    }

    #[actix_rt::test]
    async fn retries_a_midstream_drop_on_the_next_channel() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (drop_connection, wait_for_drop) = oneshot::channel::<()>();
        let (finish_server, wait_for_finish) = oneshot::channel::<()>();

        let server = tokio::spawn(async move {
            // The first channel begins a valid streaming response and then
            // loses its transport. This is the failure mode observed when a
            // load balancer reaps an idle HTTP/2 connection.
            let (socket, _) = listener.accept().await.unwrap();
            let service = service_fn(|_request: HyperRequest<Incoming>| async move {
                // One valid, empty ReadRowsResponse followed by a body that
                // never completes. The client signals only after tonic has
                // decoded this message, guaranteeing a mid-stream drop.
                let first_message = Frame::data(Bytes::from_static(&[0, 0, 0, 0, 0]));
                let body = StreamBody::new(
                    futures::stream::once(
                        async move { Ok::<Frame<Bytes>, Infallible>(first_message) },
                    )
                    .chain(futures::stream::pending::<Result<Frame<Bytes>, Infallible>>()),
                );
                Ok::<_, Infallible>(
                    HyperResponse::builder()
                        .status(200)
                        .header("content-type", "application/grpc")
                        .body(body)
                        .unwrap(),
                )
            });
            {
                let connection = http2::Builder::new(TokioExecutor::new())
                    .serve_connection(TokioIo::new(socket), service);
                tokio::pin!(connection);
                tokio::select! {
                    result = &mut connection => {
                        panic!("test HTTP/2 connection ended before it was dropped: {result:?}");
                    }
                    _ = wait_for_drop => {}
                }
            }

            // A retry must select the second channel. Return a complete,
            // successful gRPC stream there so success proves both retry
            // classification and fresh-channel selection end to end.
            let (socket, _) = listener.accept().await.unwrap();
            let service = service_fn(|_request: HyperRequest<Incoming>| async move {
                // A trailers-only success is the canonical empty gRPC
                // streaming response and ends the stream immediately.
                let body =
                    StreamBody::new(futures::stream::empty::<Result<Frame<Bytes>, Infallible>>());
                Ok::<_, Infallible>(
                    HyperResponse::builder()
                        .status(200)
                        .header("content-type", "application/grpc")
                        .header("grpc-status", "0")
                        .body(body)
                        .unwrap(),
                )
            });
            let connection = http2::Builder::new(TokioExecutor::new())
                .serve_connection(TokioIo::new(socket), service);
            tokio::pin!(connection);
            tokio::select! {
                result = &mut connection => {
                    panic!("successful HTTP/2 connection ended unexpectedly: {result:?}");
                }
                _ = wait_for_finish => {}
            }
        });

        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());
        let settings = DbSettings {
            dsn: Some(format!("grpc://localhost:{}", address.port())),
            db_settings: json!({
                "table_name": "projects/test/instances/test/tables/test",
                "grpc_channel_count": 2,
                "retry_count": 1,
                "grpc_connect_timeout": 2,
                "grpc_point_attempt_timeout": 5,
                "grpc_point_total_timeout": 10
            })
            .to_string(),
        };
        let client = BigTableClientImpl::new(metrics, &settings).unwrap();
        let drop_connection = Arc::new(Mutex::new(Some(drop_connection)));
        let policy = client.rpc_policy(RpcClass::Point, RetryKind::Read, BreakerPolicy::Track);
        let started = tokio::time::Instant::now();
        tokio::time::timeout(
            Duration::from_secs(8),
            client.execute_rpc(
                bigtable::ReadRowsRequest::default(),
                policy,
                move |channel, request| {
                    let drop_connection = drop_connection.clone();
                    async move {
                        let mut bigtable = BigtableDb::client(channel);
                        let mut stream = bigtable
                            .read_rows(request)
                            .await
                            .map_err(error::BigTableError::Read)?
                            .into_inner();

                        let signal = drop_connection
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .take();
                        if let Some(signal) = signal {
                            assert!(stream.message().await.unwrap().is_some());
                            signal.send(()).unwrap();
                        }
                        while stream
                            .message()
                            .await
                            .map_err(error::BigTableError::Read)?
                            .is_some()
                        {}
                        Ok(())
                    }
                },
            ),
        )
        .await
        .expect("Bigtable retry exceeded the test budget")
        .expect("the second channel should complete the read");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the retry waited for the attempt deadline instead of classifying the transport drop"
        );

        finish_server.send(()).unwrap();
        server.await.unwrap();
    }

    #[test]
    fn default_retry_budget_is_bounded() {
        assert_eq!(RETRY_COUNT, 2);
    }

    #[actix_rt::test]
    async fn request_preserves_metadata_and_grpc_timeout() {
        let db = BigtableDb::new(None);
        let mut metadata = MetadataMap::new();
        metadata.insert("x-goog-request-params", "table_name=test".parse().unwrap());

        let request = db
            .request(
                bigtable::ReadRowsRequest::default(),
                &metadata,
                tokio::time::Instant::now() + Duration::from_secs(5),
            )
            .await
            .unwrap();

        assert_eq!(
            request.metadata().get("x-goog-request-params").unwrap(),
            "table_name=test"
        );
        assert!(request.metadata().get("grpc-timeout").is_some());
    }

    #[actix_rt::test]
    async fn retry_metric_is_consumed_only_by_a_followup_attempt() {
        let pending = Arc::new(Mutex::new(None));
        let attempt_pending = pending.clone();
        let condition_pending = pending.clone();
        let attempts = Arc::new(AtomicUsize::new(0));
        let attempt_count = attempts.clone();
        let emitted = Arc::new(AtomicUsize::new(0));
        let emitted_count = emitted.clone();

        let result = RetryPolicy::fixed(Duration::ZERO)
            .with_max_retries(1)
            .retry_if(
                || {
                    attempt_count.fetch_add(1, Ordering::Relaxed);
                    if take_pending_retry_metric(&attempt_pending).is_some() {
                        emitted_count.fetch_add(1, Ordering::Relaxed);
                    }
                    async { Err::<(), _>(error::BigTableError::AttemptTimeout) }
                },
                move |error: &error::BigTableError| {
                    if let Some(retry_metric) = classify_retry(RetryKind::Read, error) {
                        set_pending_retry_metric(&condition_pending, retry_metric);
                        true
                    } else {
                        false
                    }
                },
            )
            .await;

        assert!(matches!(result, Err(error::BigTableError::AttemptTimeout)));
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
        assert_eq!(emitted.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn local_failures_do_not_count_toward_the_backend_breaker() {
        assert!(!counts_toward_breaker(
            &error::BigTableError::PreSendTimeout,
            false,
        ));
        assert!(!counts_toward_breaker(
            &error::BigTableError::CircuitBreakerOpen,
            false,
        ));
        assert!(!counts_toward_breaker(
            &error::BigTableError::OperationTimeout,
            false,
        ));
        assert!(counts_toward_breaker(
            &error::BigTableError::OperationTimeout,
            true,
        ));
        assert!(counts_toward_breaker(
            &error::BigTableError::Read(Status::unavailable("backend unavailable")),
            false,
        ));
    }
}

pub fn metric(metrics: &Arc<StatsdClient>, err_type: &str, code: Option<&str>) {
    let mut metric = metrics
        .incr_with_tags(MetricName::DatabaseRetry)
        .with_tag("error", err_type)
        .with_tag("type", "bigtable");
    if let Some(code) = code {
        metric = metric.with_tag("code", code);
    }
    metric.send();
}

#[derive(Clone, Copy, Debug)]
struct RetryMetric {
    error: &'static str,
    code: Option<Code>,
}

impl RetryMetric {
    fn send(self, metrics: &Arc<StatsdClient>) {
        let code = self.code.map(|code| format!("{code:?}"));
        metric(metrics, self.error, code.as_deref());
    }
}

fn take_pending_retry_metric(pending: &Mutex<Option<RetryMetric>>) -> Option<RetryMetric> {
    pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

fn set_pending_retry_metric(pending: &Mutex<Option<RetryMetric>>, metric: RetryMetric) {
    *pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(metric);
}

/// Classify a safe retry and describe the metric to emit if its next attempt
/// actually starts.
fn classify_retry(kind: RetryKind, err: &error::BigTableError) -> Option<RetryMetric> {
    debug!("🉑 Checking BigTableError...{err}");
    match err {
        error::BigTableError::InvalidRowResponse(status)
        | error::BigTableError::Read(status)
        | error::BigTableError::Write(status) => {
            let retry = match kind {
                RetryKind::Read | RetryKind::IdempotentWrite => retryable_read_err(status),
                RetryKind::ConditionalWrite => retryable_pre_send_err(status),
            };
            info!("GRPC Failure: {:?}", status);
            retry.then_some(RetryMetric {
                error: "RpcFailure",
                code: Some(status.code()),
            })
        }
        error::BigTableError::AttemptTimeout if kind != RetryKind::ConditionalWrite => {
            Some(RetryMetric {
                error: "AttemptTimeout",
                code: None,
            })
        }
        // Authentication and request construction happen before the RPC is
        // dispatched, so their deadline is safe to retry for every method,
        // including conditional mutations.
        error::BigTableError::PreSendTimeout => Some(RetryMetric {
            error: "PreSendTimeout",
            code: None,
        }),
        // Failures to fetch an OAuth token (e.g. a transient metadata server
        // hiccup) are retryable, matching grpcio's prior behavior.
        error::BigTableError::Auth(_) => Some(RetryMetric {
            error: "Auth",
            code: None,
        }),
        _ => None,
    }
}

/// Determine if a router record is "incomplete" (doesn't include [User]
/// columns):
///
/// They can be incomplete for a couple reasons:
///
/// 1) A migration code bug caused a few incomplete migrations where
///    `add_channels` and `increment_storage` calls occurred when the migration's
///    initial `add_user` was never completed:
///    https://github.com/mozilla-services/autopush-rs/pull/640
///
/// 2) When router TTLs are eventually enabled: `add_channel` and
///    `increment_storage` can write cells with later expiry times than the other
///    router cells
fn is_incomplete_router_record(cells: &RowCells) -> bool {
    cells
        .keys()
        .all(|k| ["current_timestamp", "version"].contains(&k.as_str()) || k.starts_with("chid:"))
}

/// Connect to a BigTable storage model.
///
/// BigTable is available via the Google Console, and is a schema less storage system.
///
/// The `db_dsn` string should be in the form of
/// `grpc://{BigTableEndpoint}`
///
/// The settings contains the `table_name` which is the GRPC path to the data.
/// (e.g. `projects/{project_id}/instances/{instance_id}/tables/{table_id}`)
///
/// where:
/// _BigTableEndpoint_ is the endpoint domain to use (the default is `bigtable.googleapis.com`) See
/// [BigTable Endpoints](https://cloud.google.com/bigtable/docs/regional-endpoints) for more details.
/// _project-id_ is the Google project identifier (see the Google developer console (e.g. 'autopush-dev'))
/// _instance-id_ is the Google project instance, (see the Google developer console (e.g. 'development-1'))
/// _table_id_ is the Table Name (e.g. 'autopush')
///
/// This will automatically bring in the default credentials specified by the `GOOGLE_APPLICATION_CREDENTIALS`
/// environment variable.
///
/// NOTE: Some configurations may look for the default credential file (pointed to by
/// `GOOGLE_APPLICATION_CREDENTIALS`) to be stored in
/// `$HOME/.config/gcloud/application_default_credentials.json`)
///
impl BigTableClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        debug!("🏊 BT Pool new");
        let db_settings = BigTableDbSettings::try_from(settings.db_settings.as_ref())?;
        info!("🉑 {:#?}", db_settings);
        let pool = BigTablePool::new(settings, &metrics)?;

        // create the metadata header blocks required by Google for accessing GRPC resources.
        let metadata = db_settings.metadata()?;
        Ok(Self {
            settings: db_settings,
            metrics,
            metadata,
            pool,
            circuit_breaker: Arc::new(CircuitBreaker::default()),
        })
    }

    fn router_ttl(&self) -> Duration {
        self.settings
            .max_router_ttl
            .unwrap_or(Duration::from_secs(MAX_ROUTER_TTL_SECS))
    }

    /// Spawn a task to periodically evict idle logical client handles.
    pub fn spawn_sweeper(&self, interval: Duration) {
        self.pool.spawn_sweeper(interval);
    }

    /// Return a ReadRowsRequest for a given row key
    fn read_row_request(&self, row_key: &str) -> bigtable::ReadRowsRequest {
        read_row_request(
            &self.settings.table_name,
            &self.settings.app_profile_id,
            row_key,
        )
    }

    /// Return a MutateRowRequest for a given row key
    fn mutate_row_request(&self, row_key: &str) -> bigtable::MutateRowRequest {
        bigtable::MutateRowRequest {
            table_name: self.settings.table_name.clone(),
            app_profile_id: self.settings.app_profile_id.clone(),
            row_key: row_key.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    /// Return a CheckAndMutateRowRequest for a given row key
    fn check_and_mutate_row_request(&self, row_key: &str) -> bigtable::CheckAndMutateRowRequest {
        bigtable::CheckAndMutateRowRequest {
            table_name: self.settings.table_name.clone(),
            app_profile_id: self.settings.app_profile_id.clone(),
            row_key: row_key.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    fn rpc_policy(
        &self,
        class: RpcClass,
        retry_kind: RetryKind,
        breaker: BreakerPolicy,
    ) -> RpcPolicy {
        let (attempt_timeout, total_timeout) = match class {
            RpcClass::Point => (
                self.settings.grpc_point_attempt_timeout,
                self.settings.grpc_point_total_timeout,
            ),
            RpcClass::Scan => (
                self.settings.grpc_scan_attempt_timeout,
                self.settings.grpc_scan_total_timeout,
            ),
        };
        RpcPolicy {
            attempt_timeout,
            total_timeout,
            retry_kind,
            breaker,
        }
    }

    /// Execute one logical Bigtable operation within a total budget.
    ///
    /// A deadpool checkout is a local concurrency permit. It is included in the
    /// caller's total budget, but checkout failures do not affect the Bigtable
    /// circuit breaker. Once checked out, every retry selects a fresh channel
    /// slot and rebuilds request metadata and credentials.
    async fn execute_rpc<M, T, Call, CallFuture>(
        &self,
        message: M,
        policy: RpcPolicy,
        call: Call,
    ) -> Result<T, error::BigTableError>
    where
        M: Clone,
        Call: Fn(Channel, Request<M>) -> CallFuture + Clone,
        CallFuture: Future<Output = Result<T, error::BigTableError>>,
    {
        if policy.breaker == BreakerPolicy::Track && !self.circuit_breaker.allow_request() {
            return Err(error::BigTableError::CircuitBreakerOpen);
        }

        let operation_deadline = tokio::time::Instant::now() + policy.total_timeout;
        let pooled = match tokio::time::timeout_at(operation_deadline, self.pool.get()).await {
            Ok(result) => result?,
            Err(_) => return Err(error::BigTableError::OperationTimeout),
        };
        let bigtable = (*pooled).clone();
        let metadata = self.metadata.clone();
        let retry_policy = retry_policy(self.settings.retry_count);
        let pending_retry_metric = Arc::new(Mutex::new(None));
        let attempt_retry_metric = pending_retry_metric.clone();
        let condition_retry_metric = pending_retry_metric;
        let metrics = self.metrics.clone();
        // If the total budget cancels the retry future, this remains true only
        // when cancellation happened inside a dispatched RPC. Request
        // preparation and retry backoff leave it false.
        let rpc_in_flight = Arc::new(AtomicBool::new(false));
        let attempt_rpc_in_flight = rpc_in_flight.clone();
        let result = operation_budget_until(
            operation_deadline,
            retry_policy.retry_if(
                || {
                    // The retry predicate runs even for a terminal failure.
                    // Emit only when the next attempt actually starts.
                    if let Some(retry_metric) = take_pending_retry_metric(&attempt_retry_metric) {
                        retry_metric.send(&metrics);
                    }
                    // The final attempt may start near the end of the total
                    // retry budget. Clamp its grpc-timeout to that remaining
                    // budget so the server does not keep abandoned work alive
                    // after this logical operation has already returned.
                    let attempt_deadline = std::cmp::min(
                        tokio::time::Instant::now() + policy.attempt_timeout,
                        operation_deadline,
                    );
                    let bigtable = bigtable.clone();
                    let metadata = metadata.clone();
                    let message = message.clone();
                    let channel = self.pool.next_channel();
                    let call = call.clone();
                    let rpc_in_flight = attempt_rpc_in_flight.clone();
                    async move {
                        let request = bigtable
                            .request(message, &metadata, attempt_deadline)
                            .await?;
                        rpc_in_flight.store(true, Ordering::Relaxed);
                        let result =
                            rpc_attempt_until(attempt_deadline, call(channel, request)).await;
                        rpc_in_flight.store(false, Ordering::Relaxed);
                        result
                    }
                },
                move |error: &error::BigTableError| {
                    if let Some(retry_metric) = classify_retry(policy.retry_kind, error) {
                        set_pending_retry_metric(&condition_retry_metric, retry_metric);
                        true
                    } else {
                        false
                    }
                },
            ),
        )
        .await;

        if policy.breaker == BreakerPolicy::Track {
            let operation_timed_out_in_rpc = rpc_in_flight.load(Ordering::Relaxed);
            match &result {
                Ok(_) => self.circuit_breaker.record_success(),
                Err(error) if counts_toward_breaker(error, operation_timed_out_in_rpc) => {
                    self.circuit_breaker.record_failure();
                }
                Err(_) => {}
            }
        }
        result
    }

    /// Apply an idempotent mutation to one row.
    async fn mutate_row(
        &self,
        req: bigtable::MutateRowRequest,
    ) -> Result<(), error::BigTableError> {
        let policy = self.rpc_policy(
            RpcClass::Point,
            RetryKind::IdempotentWrite,
            BreakerPolicy::Track,
        );
        self.execute_rpc(req, policy, |channel, request| async move {
            let mut client = BigtableDb::client(channel);
            client
                .mutate_row(request)
                .await
                .map_err(error::BigTableError::Write)?;
            Ok(())
        })
        .await
    }

    /// Read one row for the [ReadRowsRequest] (assuming only a single row was requested).
    async fn read_row(
        &self,
        req: bigtable::ReadRowsRequest,
    ) -> Result<Option<row::Row>, error::BigTableError> {
        let mut rows = self.read_rows_with_class(req, RpcClass::Point).await?;
        Ok(rows.pop_first().map(|(_, v)| v))
    }

    /// Take a big table ReadRowsRequest (containing the keys and filters) and return a set of row data indexed by row key.
    ///
    ///
    async fn read_rows(
        &self,
        req: bigtable::ReadRowsRequest,
    ) -> Result<BTreeMap<RowKey, row::Row>, error::BigTableError> {
        self.read_rows_with_class(req, RpcClass::Scan).await
    }

    async fn read_rows_with_class(
        &self,
        req: bigtable::ReadRowsRequest,
        class: RpcClass,
    ) -> Result<BTreeMap<RowKey, row::Row>, error::BigTableError> {
        self.read_rows_with_policy(req, class, BreakerPolicy::Track)
            .await
    }

    async fn read_rows_with_policy(
        &self,
        req: bigtable::ReadRowsRequest,
        class: RpcClass,
        breaker: BreakerPolicy,
    ) -> Result<BTreeMap<RowKey, row::Row>, error::BigTableError> {
        let policy = self.rpc_policy(class, RetryKind::Read, breaker);
        self.execute_rpc(req, policy, |channel, request| async move {
            let mut client = BigtableDb::client(channel);
            let response = client
                .read_rows(request)
                .await
                .map_err(error::BigTableError::Read)?
                .into_inner();
            merge::RowMerger::process_chunks(response).await
        })
        .await
    }

    /// write a given row.
    ///
    /// there's also `.mutate_rows` which I presume allows multiple.
    async fn write_row(&self, row: row::Row) -> Result<(), error::BigTableError> {
        let mut req = self.mutate_row_request(&row.row_key);
        // compile the mutations.
        // It's possible to do a lot here, including altering in process
        // mutations, clearing them, etc. It's all up for grabs until we commit
        // below. For now, let's just presume a write and be done.
        req.mutations = self.get_mutations(row.cells)?;
        self.mutate_row(req).await?;
        Ok(())
    }

    /// Compile the list of mutations for this row.
    fn get_mutations(
        &self,
        cells: HashMap<FamilyId, Vec<crate::db::bigtable::bigtable_client::cell::Cell>>,
    ) -> Result<Vec<bigtable::Mutation>, error::BigTableError> {
        let mut mutations = Vec::new();
        for (family_id, cells) in cells {
            for cell in cells {
                let timestamp = cell
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(error::BigTableError::WriteTime)?;
                debug!("🉑 expiring in {:?}", timestamp.as_millis());
                mutations.push(bigtable::Mutation {
                    mutation: Some(bigtable::mutation::Mutation::SetCell(
                        bigtable::mutation::SetCell {
                            family_name: family_id.clone(),
                            column_qualifier: cell.qualifier.clone().into_bytes(),
                            // Bigtable tables use millisecond cell-timestamp granularity;
                            // timestamp_micros must be a multiple of 1,000 or the server
                            // rejects the mutation with a granularity mismatch.
                            timestamp_micros: (timestamp.as_millis() * 1000) as i64,
                            value: cell.value,
                        },
                    )),
                });
            }
        }
        Ok(mutations)
    }

    /// Write mutations if the row meets a condition specified by the filter.
    ///
    /// Mutations can be applied either when the filter matches (state `true`)
    /// or doesn't match (state `false`).
    ///
    /// Returns whether the filter matched records (which indicates whether the
    /// mutations were applied, depending on the state)
    async fn check_and_mutate_row(
        &self,
        row: row::Row,
        filter: bigtable::RowFilter,
        state: bool,
    ) -> Result<bool, error::BigTableError> {
        let mut req = self.check_and_mutate_row_request(&row.row_key);
        let mutations = self.get_mutations(row.cells)?;
        req.predicate_filter = Some(filter);
        if state {
            req.true_mutations = mutations;
        } else {
            req.false_mutations = mutations;
        }
        self.check_and_mutate(req).await
    }

    async fn check_and_mutate(
        &self,
        req: bigtable::CheckAndMutateRowRequest,
    ) -> Result<bool, error::BigTableError> {
        let policy = self.rpc_policy(
            RpcClass::Point,
            RetryKind::ConditionalWrite,
            BreakerPolicy::Track,
        );
        let predicate_matched = self
            .execute_rpc(req, policy, |channel, request| async move {
                let mut client = BigtableDb::client(channel);
                let response = client
                    .check_and_mutate_row(request)
                    .await
                    .map_err(error::BigTableError::Write)?;
                Ok(response.into_inner().predicate_matched)
            })
            .await?;
        debug!("🉑 Predicate Matched: {predicate_matched}");
        Ok(predicate_matched)
    }

    fn get_delete_mutations(
        &self,
        family: &str,
        column_names: &[&str],
        time_range: Option<&bigtable::TimestampRange>,
    ) -> Result<Vec<bigtable::Mutation>, error::BigTableError> {
        let mut mutations = Vec::new();
        for column in column_names {
            // DeleteFromRow -- Delete all cells for a given row.
            // DeleteFromFamily -- Delete all cells from a family for a given row.
            // DeleteFromColumn -- Delete all cells from a column name for a given row, restricted by timestamp range.
            mutations.push(bigtable::Mutation {
                mutation: Some(bigtable::mutation::Mutation::DeleteFromColumn(
                    bigtable::mutation::DeleteFromColumn {
                        family_name: family.to_owned(),
                        column_qualifier: column.as_bytes().to_vec(),
                        time_range: time_range.cloned(),
                    },
                )),
            });
        }
        Ok(mutations)
    }

    /// Delete all the cells for the given row. NOTE: This will drop the row.
    async fn delete_row(&self, row_key: &str) -> Result<(), error::BigTableError> {
        let mut req = self.mutate_row_request(row_key);
        req.mutations = vec![bigtable::Mutation {
            mutation: Some(bigtable::mutation::Mutation::DeleteFromRow(
                bigtable::mutation::DeleteFromRow {},
            )),
        }];
        self.mutate_row(req).await
    }

    fn rows_to_notifications(
        &self,
        rows: BTreeMap<String, Row>,
    ) -> Result<Vec<Notification>, DbError> {
        rows.into_iter()
            .map(|(row_key, row)| self.row_to_notification(&row_key, row))
            .collect()
    }

    fn row_to_notification(&self, row_key: &str, mut row: Row) -> Result<Notification, DbError> {
        let Some((_, chidmessageid)) = row_key.split_once('#') else {
            return Err(DbError::Integrity(
                "rows_to_notification expected row_key: uaid:chidmessageid ".to_owned(),
                None,
            ));
        };
        let range_key = RangeKey::parse_chidmessageid(chidmessageid).map_err(|e| {
            DbError::Integrity(
                format!("rows_to_notification expected chidmessageid: {e}"),
                None,
            )
        })?;

        // Create from the known, required fields.
        let mut notif = Notification {
            channel_id: range_key.channel_id,
            topic: range_key.topic,
            sortkey_timestamp: range_key.sortkey_timestamp,
            version: to_string(row.take_required_cell("version")?.value, "version")?,
            ttl: to_u64(row.take_required_cell("ttl")?.value, "ttl")?,
            timestamp: to_u64(row.take_required_cell("timestamp")?.value, "timestamp")?,
            ..Default::default()
        };

        // Backfill the Optional fields
        if let Some(cell) = row.take_cell("data") {
            notif.data = Some(to_string(cell.value, "data")?);
        }
        #[cfg(feature = "reliable_report")]
        {
            if let Some(cell) = row.take_cell("reliability_id") {
                notif.reliability_id = Some(to_string(cell.value, "reliability_id")?);
            }
            if let Some(cell) = row.take_cell("reliable_state") {
                notif.reliable_state = Some(
                    crate::reliability::ReliabilityState::from_str(&to_string(
                        cell.value,
                        "reliable_state",
                    )?)
                    .map_err(|e| {
                        DbError::DeserializeString(format!("Could not parse reliable_state {e:?}"))
                    })?,
                );
            }
        }
        if let Some(cell) = row.take_cell("headers") {
            notif.headers = Some(
                serde_json::from_str::<HashMap<String, String>>(&to_string(cell.value, "headers")?)
                    .map_err(|e| DbError::Serialization(e.to_string()))?,
            );
        }
        #[cfg(feature = "reliable_report")]
        if let Some(cell) = row.take_cell("reliability_id") {
            trace!("🚣  Is reliable");
            notif.reliability_id = Some(to_string(cell.value, "reliability_id")?);
        }

        trace!("🚣  Deserialized message row: {:?}", &notif);
        Ok(notif)
    }

    /// Return a Row for writing from a [User] and a `version`
    ///
    /// `version` is specified as an argument (ignoring [User::version]) so
    /// that [update_user] may specify a new version to write before modifying
    /// the [User] struct
    fn user_to_row(&self, user: &User, version: &Uuid) -> Row {
        let row_key = user.uaid.simple().to_string();
        let mut row = Row::new(row_key);
        let expiry = std::time::SystemTime::now() + self.router_ttl();

        let mut cells: Vec<cell::Cell> = vec![
            cell::Cell {
                qualifier: "connected_at".to_owned(),
                value: user.connected_at.to_be_bytes().to_vec(),
                timestamp: expiry,
                ..Default::default()
            },
            cell::Cell {
                qualifier: "router_type".to_owned(),
                value: user.router_type.clone().into_bytes(),
                timestamp: expiry,
                ..Default::default()
            },
            cell::Cell {
                qualifier: "record_version".to_owned(),
                value: user
                    .record_version
                    .unwrap_or(USER_RECORD_VERSION)
                    .to_be_bytes()
                    .to_vec(),
                timestamp: expiry,
                ..Default::default()
            },
            cell::Cell {
                qualifier: "version".to_owned(),
                value: (*version).into(),
                timestamp: expiry,
                ..Default::default()
            },
        ];

        if let Some(router_data) = &user.router_data {
            cells.push(cell::Cell {
                qualifier: "router_data".to_owned(),
                value: json!(router_data).to_string().as_bytes().to_vec(),
                timestamp: expiry,
                ..Default::default()
            });
        };
        if let Some(current_timestamp) = user.current_timestamp {
            cells.push(cell::Cell {
                qualifier: "current_timestamp".to_owned(),
                value: current_timestamp.to_be_bytes().to_vec(),
                timestamp: expiry,
                ..Default::default()
            });
        };
        if let Some(node_id) = &user.node_id {
            cells.push(cell::Cell {
                qualifier: "node_id".to_owned(),
                value: node_id.as_bytes().to_vec(),
                timestamp: expiry,
                ..Default::default()
            });
        };

        cells.extend(channels_to_cells(
            Cow::Borrowed(&user.priv_channels),
            expiry,
        ));

        row.add_cells(ROUTER_FAMILY, cells);
        row
    }
}

#[derive(Clone)]
pub struct BigtableDb {
    /// Application Default Credentials token provider, shared across the
    /// pool. `None` when running against the emulator (which needs no
    /// credentials).
    auth_provider: Option<Arc<dyn TokenProvider>>,
}

impl BigtableDb {
    pub fn new(auth_provider: Option<Arc<dyn TokenProvider>>) -> Self {
        Self { auth_provider }
    }

    pub(super) fn client(channel: Channel) -> BigtableClient<Channel> {
        BigtableClient::new(channel)
            .max_decoding_message_size(MAX_MESSAGE_LEN)
            .max_encoding_message_size(MAX_MESSAGE_LEN)
    }

    /// Build a [tonic::Request] for the given message, attaching the standard
    /// Google routing metadata and (outside of the emulator) an OAuth2
    /// `authorization` token from the Application Default Credentials.
    ///
    /// The token provider caches tokens internally, so this is cheap to call
    /// per-request (and per retry attempt, where it transparently picks up a
    /// fresh token if the prior one expired). Token preparation is bounded by
    /// `deadline`; the remaining time is also sent to Bigtable as the
    /// `grpc-timeout` metadata value. A local deadline expiry is reported as
    /// [error::BigTableError::PreSendTimeout].
    pub(super) async fn request<T>(
        &self,
        msg: T,
        metadata: &MetadataMap,
        deadline: tokio::time::Instant,
    ) -> Result<Request<T>, error::BigTableError> {
        let authorization = if let Some(provider) = &self.auth_provider {
            let token = tokio::time::timeout_at(deadline, provider.token(BIGTABLE_DATA_SCOPES))
                .await
                .map_err(|_| error::BigTableError::PreSendTimeout)?
                .map_err(error::BigTableError::Auth)?;
            Some(
                format!("Bearer {}", token.as_str())
                    .parse::<AsciiMetadataValue>()
                    .map_err(|e| {
                        error::BigTableError::Config(format!("Invalid auth token: {e}"))
                    })?,
            )
        } else {
            None
        };

        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(error::BigTableError::PreSendTimeout);
        }

        let mut request = Request::new(msg);
        *request.metadata_mut() = metadata.clone();
        // Set this after copying metadata: set_timeout is implemented by
        // inserting the grpc-timeout entry into the request metadata map.
        request.set_timeout(remaining);
        if let Some(value) = authorization {
            request.metadata_mut().insert("authorization", value);
        }
        Ok(request)
    }
}

#[async_trait]
impl DbClient for BigTableClientImpl {
    /// add user to the database
    async fn add_user(&self, user: &User) -> DbResult<()> {
        trace!("🉑 Adding user");
        let Some(ref version) = user.version else {
            return Err(DbError::General(
                "add_user expected a user version field".to_owned(),
            ));
        };
        let row = self.user_to_row(user, version);

        // Only add when the user doesn't already exist
        let row_key_filter = bigtable::RowFilter {
            filter: Some(bigtable::row_filter::Filter::RowKeyRegexFilter(
                format!("^{}$", row.row_key).into_bytes(),
            )),
        };
        let filter = filter_chain(vec![router_gc_policy_filter(), row_key_filter]);

        if self.check_and_mutate_row(row, filter, false).await? {
            return Err(DbError::Conditional);
        }
        Ok(())
    }

    /// BigTable doesn't really have the concept of an "update". You simply write the data and
    /// the individual cells create a new version. Depending on the garbage collection rules for
    /// the family, these can either persist or be automatically deleted.
    ///
    /// NOTE: This function updates the key ROUTER records for a given UAID. It does this by
    /// calling [BigTableClientImpl::user_to_row] which creates a new row with new `cell.timestamp` values set
    /// to now + `MAX_ROUTER_TTL`. This function is called by mobile during the daily
    /// [autoendpoint::routes::update_token_route] handling, and by desktop
    /// [autoconnect-ws-sm::get_or_create_user]` which is called
    /// during the `HELLO` handler. This should be enough to ensure that the ROUTER records
    /// are properly refreshed for "lively" clients.
    ///
    /// NOTE: There is some, very small, potential risk that a desktop client that can
    /// somehow remain connected the duration of MAX_ROUTER_TTL, may be dropped as not being
    /// "lively".
    async fn update_user(&self, user: &mut User) -> DbResult<bool> {
        let Some(ref version) = user.version else {
            return Err(DbError::General(
                "update_user expected a user version field".to_owned(),
            ));
        };

        let mut filters = vec![router_gc_policy_filter()];
        filters.extend(version_filter(version));
        let filter = filter_chain(filters);

        let new_version = Uuid::new_v4();
        // Always write a newly generated version
        let row = self.user_to_row(user, &new_version);

        let predicate_matched = self.check_and_mutate_row(row, filter, true).await?;
        user.version = Some(new_version);
        Ok(predicate_matched)
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let row_key = uaid.as_simple().to_string();
        let mut req = self.read_row_request(&row_key);
        let mut filters = vec![router_gc_policy_filter()];
        filters.push(family_filter(format!("^{ROUTER_FAMILY}$")));
        req.filter = Some(filter_chain(filters));
        let Some(mut row) = self.read_row(req).await? else {
            return Ok(None);
        };

        trace!("🉑 Found a record for {}", row_key);

        let connected_at_cell = match row.take_required_cell("connected_at") {
            Ok(cell) => cell,
            Err(_) => {
                if !is_incomplete_router_record(&row.cells) {
                    return Err(DbError::Integrity(
                        "Expected column: connected_at".to_owned(),
                        Some(format!("{row:#?}")),
                    ));
                }
                // Special case incomplete records: they're equivalent to no
                // user exists. Incompletes caused by the migration bug in #640
                // will have their migration re-triggered by returning None:
                // https://github.com/mozilla-services/autopush-rs/pull/640
                trace!("🉑 Dropping an incomplete user record for {}", row_key);
                self.metrics
                    .incr_with_tags(MetricName::DatabaseDropUser)
                    .with_tag("reason", "incomplete_record")
                    .send();
                self.remove_user(uaid).await?;
                return Ok(None);
            }
        };

        let mut result = User {
            uaid: *uaid,
            connected_at: to_u64(connected_at_cell.value, "connected_at")?,
            router_type: to_string(row.take_required_cell("router_type")?.value, "router_type")?,
            record_version: Some(to_u64(
                row.take_required_cell("record_version")?.value,
                "record_version",
            )?),
            version: Some(
                row.take_required_cell("version")?
                    .value
                    .try_into()
                    .map_err(|e| {
                        DbError::Serialization(format!("Could not deserialize version: {e:?}"))
                    })?,
            ),
            ..Default::default()
        };

        if let Some(cell) = row.take_cell("router_data") {
            result.router_data = from_str(&to_string(cell.value, "router_type")?).map_err(|e| {
                DbError::Serialization(format!("Could not deserialize router_type: {e:?}"))
            })?;
        }

        if let Some(cell) = row.take_cell("node_id") {
            result.node_id = Some(to_string(cell.value, "node_id")?);
        }

        if let Some(cell) = row.take_cell("current_timestamp") {
            result.current_timestamp = Some(to_u64(cell.value, "current_timestamp")?)
        }

        // Read the channels last, after removal of all non channel cells
        result.priv_channels = channels_from_cells(&row.cells)?;

        Ok(Some(result))
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let row_key = uaid.simple().to_string();
        self.delete_row(&row_key).await?;
        Ok(())
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let channels = HashSet::from_iter([channel_id.to_owned()]);
        self.add_channels(uaid, channels).await
    }

    /// Add channels in bulk (used mostly during migration)
    ///
    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        // channel_ids are stored as a set within one Bigtable row
        //
        // Bigtable allows "millions of columns in a table, as long as no row
        // exceeds the maximum limit of 256 MB per row" enabling the use of
        // column qualifiers as data.
        //
        // The "set" of channel_ids consists of column qualifiers named
        // "chid:<chid value>" as set member entries (with their cell values
        // being a single 0 byte).
        //
        // Storing the full set in a single row makes batch updates
        // (particularly to reset the GC expiry timestamps) potentially more
        // easy/efficient
        let row_key = uaid.simple().to_string();
        let mut row = Row::new(row_key);
        let expiry = std::time::SystemTime::now() + self.router_ttl();

        // Note: updating the version column isn't necessary here because this
        // write only adds a new (or updates an existing) column with a 0 byte
        // value
        row.add_cells(
            ROUTER_FAMILY,
            channels_to_cells(Cow::Owned(channels), expiry),
        );

        self.write_row(row).await?;
        Ok(())
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let row_key = uaid.simple().to_string();
        let mut req = self.read_row_request(&row_key);

        let cq_filter = bigtable::RowFilter {
            filter: Some(bigtable::row_filter::Filter::ColumnQualifierRegexFilter(
                "^chid:.*$".as_bytes().to_vec(),
            )),
        };
        req.filter = Some(filter_chain(vec![
            router_gc_policy_filter(),
            family_filter(format!("^{ROUTER_FAMILY}$")),
            cq_filter,
        ]));

        let Some(row) = self.read_row(req).await? else {
            return Ok(Default::default());
        };
        channels_from_cells(&row.cells)
    }

    /// Delete the channel. Does not delete its associated pending messages.
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let row_key = uaid.simple().to_string();
        let mut req = self.check_and_mutate_row_request(&row_key);

        // Delete the column representing the channel_id
        let column = format!("chid:{}", channel_id.as_hyphenated());
        let mut mutations = self.get_delete_mutations(ROUTER_FAMILY, &[column.as_ref()], None)?;

        // and write a new version cell
        let mut row = Row::new(row_key);
        let expiry = std::time::SystemTime::now() + self.router_ttl();
        row.cells
            .insert(ROUTER_FAMILY.to_owned(), vec![new_version_cell(expiry)]);
        mutations.extend(self.get_mutations(row.cells)?);

        // check if the channel existed/was actually removed
        let cq_filter = bigtable::RowFilter {
            filter: Some(bigtable::row_filter::Filter::ColumnQualifierRegexFilter(
                format!("^{column}$").into_bytes(),
            )),
        };
        req.predicate_filter = Some(filter_chain(vec![router_gc_policy_filter(), cq_filter]));
        req.true_mutations = mutations;

        Ok(self.check_and_mutate(req).await?)
    }

    /// Remove the node_id
    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        _node_id: &str,
        _connected_at: u64,
        version: &Option<Uuid>,
    ) -> DbResult<bool> {
        let row_key = uaid.simple().to_string();
        trace!("🉑 Removing node_id for: {row_key} (version: {version:?}) ",);
        let Some(version) = version else {
            return Err(DbError::General("Expected a user version field".to_owned()));
        };

        let mut req = self.check_and_mutate_row_request(&row_key);

        let mut filters = vec![router_gc_policy_filter()];
        filters.extend(version_filter(version));
        req.predicate_filter = Some(filter_chain(filters));
        req.true_mutations = self.get_delete_mutations(ROUTER_FAMILY, &["node_id"], None)?;

        Ok(self.check_and_mutate(req).await?)
    }

    /// Write the notification to storage.
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let row_key = format!("{}#{}", uaid.simple(), message.chidmessageid());
        debug!("🗄️ Saving message {} :: {:?}", &row_key, &message);
        trace!(
            "🉑 timestamp: {:?}",
            &message.timestamp.to_be_bytes().to_vec()
        );
        let mut row = Row::new(row_key);

        // Remember, `timestamp` is effectively the time to kill the message, not the
        // current time.
        // TODO: use message.expiry()
        let expiry = SystemTime::now() + Duration::from_secs(message.ttl);
        trace!(
            "🉑 Message Expiry {}",
            expiry
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        let mut cells: Vec<cell::Cell> = Vec::new();

        let is_topic = message.topic.is_some();
        let family = if is_topic {
            MESSAGE_TOPIC_FAMILY
        } else {
            MESSAGE_FAMILY
        };
        cells.extend(vec![
            cell::Cell {
                qualifier: "ttl".to_owned(),
                value: message.ttl.to_be_bytes().to_vec(),
                timestamp: expiry,
                ..Default::default()
            },
            cell::Cell {
                qualifier: "timestamp".to_owned(),
                value: message.timestamp.to_be_bytes().to_vec(),
                timestamp: expiry,
                ..Default::default()
            },
            cell::Cell {
                qualifier: "version".to_owned(),
                value: message.version.into_bytes(),
                timestamp: expiry,
                ..Default::default()
            },
        ]);
        if let Some(headers) = message.headers
            && !headers.is_empty()
        {
            cells.push(cell::Cell {
                qualifier: "headers".to_owned(),
                value: json!(headers).to_string().into_bytes(),
                timestamp: expiry,
                ..Default::default()
            });
        }
        #[cfg(feature = "reliable_report")]
        {
            if let Some(reliability_id) = message.reliability_id {
                trace!("🔍 FOUND RELIABILITY ID: {}", reliability_id);
                cells.push(cell::Cell {
                    qualifier: "reliability_id".to_owned(),
                    value: reliability_id.into_bytes(),
                    timestamp: expiry,
                    ..Default::default()
                });
            }
            if let Some(reliable_state) = message.reliable_state {
                cells.push(cell::Cell {
                    qualifier: "reliable_state".to_owned(),
                    value: reliable_state.to_string().into_bytes(),
                    timestamp: expiry,
                    ..Default::default()
                });
            }
        }
        if let Some(data) = message.data {
            cells.push(cell::Cell {
                qualifier: "data".to_owned(),
                value: data.into_bytes(),
                timestamp: expiry,
                ..Default::default()
            });
        }

        row.add_cells(family, cells);
        trace!("🉑 Adding row");
        self.write_row(row).await?;

        self.metrics
            .incr_with_tags(MetricName::NotificationMessageStored)
            .with_tag("topic", &is_topic.to_string())
            .with_tag("database", &self.name())
            .send();
        Ok(())
    }

    /// Save a batch of messages to the database.
    ///
    /// Currently just iterating through the list and saving one at a time. There's a bulk way
    /// to save messages, but there are other considerations (e.g. mutation limits)
    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        // plate simple way of solving this:
        for message in messages {
            self.save_message(uaid, message).await?;
        }
        Ok(())
    }

    /// Set the `current_timestamp` in the meta record for this user agent.
    ///
    /// This is a bit different for BigTable. Field expiration (technically cell
    /// expiration) is determined by the lifetime assigned to the cell once it hits
    /// a given date. That means you can't really extend a lifetime by adjusting a
    /// single field. You'd have to adjust all the cells that are in the family.
    /// So, we're not going to do expiration that way.
    ///
    /// That leaves the meta "current_timestamp" field. We do not purge ACK'd records,
    /// instead we presume that the TTL will kill them off eventually. On reads, we use
    /// the `current_timestamp` to determine what records to return, since we return
    /// records with timestamps later than `current_timestamp`.
    ///
    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        let row_key = uaid.simple().to_string();
        debug!(
            "🉑 Updating {} current_timestamp:  {:?}",
            &row_key,
            timestamp.to_be_bytes().to_vec()
        );
        let expiry = std::time::SystemTime::now() + self.router_ttl();
        let mut row = Row::new(row_key.clone());

        row.cells.insert(
            ROUTER_FAMILY.to_owned(),
            vec![
                cell::Cell {
                    qualifier: "current_timestamp".to_owned(),
                    value: timestamp.to_be_bytes().to_vec(),
                    timestamp: expiry,
                    ..Default::default()
                },
                new_version_cell(expiry),
            ],
        );

        self.write_row(row).await?;

        Ok(())
    }

    /// Delete the notification from storage.
    async fn remove_message(&self, uaid: &Uuid, chidmessageid: &str) -> DbResult<()> {
        trace!(
            "🉑 attemping to delete {:?} :: {:?}",
            uaid.to_string(),
            chidmessageid
        );
        let row_key = format!("{}#{}", uaid.simple(), chidmessageid);
        debug!("🉑🔥 Deleting message {}", &row_key);
        self.delete_row(&row_key).await?;
        self.metrics
            .incr_with_tags(MetricName::NotificationMessageDeleted)
            .with_tag("database", &self.name())
            .send();
        Ok(())
    }

    /// Return `limit` pending messages from storage. `limit=0` for all messages.
    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let start_key = format!("{}#01:", uaid.simple());
        let end_key = format!("{}#02:", uaid.simple());
        let mut req = bigtable::ReadRowsRequest {
            table_name: self.settings.table_name.clone(),
            app_profile_id: self.settings.app_profile_id.clone(),
            rows: Some(bigtable::RowSet {
                row_keys: Vec::new(),
                row_ranges: vec![bigtable::RowRange {
                    start_key: Some(bigtable::row_range::StartKey::StartKeyOpen(
                        start_key.into_bytes(),
                    )),
                    end_key: Some(bigtable::row_range::EndKey::EndKeyOpen(
                        end_key.into_bytes(),
                    )),
                }],
            }),
            ..Default::default()
        };

        let mut filters = message_gc_policy_filter()?;
        filters.push(family_filter(format!("^{MESSAGE_TOPIC_FAMILY}$")));

        req.filter = Some(filter_chain(filters));
        if limit > 0 {
            trace!("🉑 Setting limit to {limit}");
            req.rows_limit = limit as i64;
        }
        let rows = self.read_rows(req).await?;
        debug!(
            "🉑 Fetch Topic Messages. Found {} row(s) of {}",
            rows.len(),
            limit
        );

        let messages = self.rows_to_notifications(rows)?;

        // Note: Bigtable always returns a timestamp of None.
        // Under Bigtable `current_timestamp` is instead initially read
        // from [get_user].
        Ok(FetchMessageResponse {
            messages,
            timestamp: None,
        })
    }

    /// Return `limit` messages pending for a UAID that have a sortkey_timestamp after
    /// what's specified. `limit=0` for all messages.
    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let start_key = if let Some(ts) = timestamp {
            // Fetch everything after the last message with timestamp: the "z"
            // moves past the last message's channel_id's 1st hex digit
            format!("{}#02:{}z", uaid.simple(), ts)
        } else {
            format!("{}#02:", uaid.simple())
        };
        let end_key = format!("{}#03:", uaid.simple());
        let mut req = bigtable::ReadRowsRequest {
            table_name: self.settings.table_name.clone(),
            app_profile_id: self.settings.app_profile_id.clone(),
            rows: Some(bigtable::RowSet {
                row_keys: Vec::new(),
                row_ranges: vec![bigtable::RowRange {
                    start_key: Some(bigtable::row_range::StartKey::StartKeyOpen(
                        start_key.into_bytes(),
                    )),
                    end_key: Some(bigtable::row_range::EndKey::EndKeyOpen(
                        end_key.into_bytes(),
                    )),
                }],
            }),
            ..Default::default()
        };

        // We can fetch data and do [some remote filtering](https://cloud.google.com/bigtable/docs/filters),
        // unfortunately I don't think the filtering we need will be super helpful.
        //
        //
        /*
        //NOTE: if you filter on a given field, BigTable will only
        // return that specific field. Adding filters for the rest of
        // the known elements may NOT return those elements or may
        // cause the message to not be returned because any of
        // those elements are not present. It may be preferable to
        // therefore run two filters, one to fetch the candidate IDs
        // and another to fetch the content of the messages.
         */
        let mut filters = message_gc_policy_filter()?;
        filters.push(family_filter(format!("^{MESSAGE_FAMILY}$")));

        req.filter = Some(filter_chain(filters));
        if limit > 0 {
            req.rows_limit = limit as i64;
        }
        let rows = self.read_rows(req).await?;
        debug!(
            "🉑 Fetch Timestamp Messages ({:?}) Found {} row(s) of {}",
            timestamp,
            rows.len(),
            limit,
        );

        let messages = self.rows_to_notifications(rows)?;
        // The timestamp of the last message read
        let timestamp = messages.last().and_then(|m| m.sortkey_timestamp);
        Ok(FetchMessageResponse {
            messages,
            timestamp,
        })
    }

    async fn health_check(&self) -> DbResult<bool> {
        // Use a random key so health checks do not create a hot tablet. The
        // BlockAll filter verifies the data path without returning row data.
        let random_uaid = Uuid::new_v4().simple().to_string();
        let mut req = read_row_request(
            &self.settings.table_name,
            &self.settings.app_profile_id,
            &random_uaid,
        );
        req.filter = Some(bigtable::RowFilter {
            filter: Some(bigtable::row_filter::Filter::BlockAllFilter(true)),
        });
        // Health is an independent, bounded backend probe. It must neither be
        // blocked by nor mutate the application traffic circuit breaker.
        self.read_rows_with_policy(req, RpcClass::Point, BreakerPolicy::Ignore)
            .await?;
        Ok(true)
    }

    /// Returns true, because there's only one table in BigTable. We divide things up
    /// by `family`.
    async fn router_table_exists(&self) -> DbResult<bool> {
        Ok(true)
    }

    /// Returns true, because there's only one table in BigTable. We divide things up
    /// by `family`.
    async fn message_table_exists(&self) -> DbResult<bool> {
        Ok(true)
    }

    #[cfg(feature = "reliable_report")]
    async fn log_report(
        &self,
        reliability_id: &str,
        new_state: crate::reliability::ReliabilityState,
    ) -> DbResult<()> {
        let row_key = reliability_id.to_owned();

        let mut row = Row::new(row_key);
        let expiry = SystemTime::now() + Duration::from_secs(RELIABLE_LOG_TTL.num_seconds() as u64);

        // Log the latest transition time for this id.
        let cells: Vec<cell::Cell> = vec![cell::Cell {
            qualifier: new_state.to_string(),
            value: crate::util::ms_since_epoch().to_be_bytes().to_vec(),
            timestamp: expiry,
            ..Default::default()
        }];

        row.add_cells(RELIABLE_LOG_FAMILY, cells);

        self.write_row(row).await?;

        Ok(())
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }

    fn name(&self) -> String {
        "Bigtable".to_owned()
    }

    fn pool_status(&self) -> Option<deadpool::Status> {
        Some(self.pool.pool.status())
    }

    fn configured_channel_count(&self) -> Option<usize> {
        Some(self.pool.configured_channel_count())
    }
}

#[cfg(all(test, feature = "emulator"))]
mod tests {

    //! Currently, these test rely on having a BigTable emulator running on the current machine.
    //! The tests presume to be able to connect to localhost:8086. See docs/bigtable.md for
    //! details and how to set up and initialize an emulator.
    //!
    use std::sync::Arc;
    use std::time::SystemTime;

    use cadence::StatsdClient;
    use uuid;

    use super::*;
    use crate::{db::DbSettings, test_support::gen_test_uaid, util::ms_since_epoch};

    const TEST_USER: &str = "DEADBEEF-0000-0000-0000-0123456789AB";
    const TEST_CHID: &str = "DECAFBAD-0000-0000-0000-0123456789AB";
    const TOPIC_CHID: &str = "DECAFBAD-1111-0000-0000-0123456789AB";

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn new_client() -> DbResult<BigTableClientImpl> {
        let env_dsn = format!(
            "grpc://{}",
            std::env::var("BIGTABLE_EMULATOR_HOST").unwrap_or("localhost:8080".to_owned())
        );
        let settings = DbSettings {
            // this presumes the table was created with
            // ```
            // scripts/setup_bt.sh
            // ```
            // with `message`, `router`, and `message_topic` families
            dsn: Some(env_dsn),
            db_settings: json!({"table_name": "projects/test/instances/test/tables/autopush"})
                .to_string(),
        };

        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());

        BigTableClientImpl::new(metrics, &settings)
    }

    #[test]
    fn escape_bytes_for_regex() {
        let b = b"hi";
        assert_eq!(escape_bytes(b), b.to_vec());
        assert_eq!(escape_bytes(b"h.*i!"), b"h\\.\\*i\\!".to_vec());
        let b = b"\xe2\x80\xb3";
        assert_eq!(escape_bytes(b), b.to_vec());
        // clippy::octal-escapes rightly discourages this ("\022") in a byte literal
        let b = [b'f', b'o', b'\0', b'2', b'2', b'o'];
        assert_eq!(escape_bytes(&b), b"fo\\x0022o".to_vec());
        let b = b"\xc0";
        assert_eq!(escape_bytes(b), b.to_vec());
        assert_eq!(escape_bytes(b"\x03"), b"\\\x03".to_vec());
    }

    #[actix_rt::test]
    async fn health_check() {
        let client = new_client().unwrap();

        let result = client.health_check().await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    /// Bigtable rejects SetCell timestamps that are not aligned to the table's
    /// millisecond granularity (timestamp_micros must be a multiple of 1,000).
    #[actix_rt::test]
    async fn timestamp_granularity_rejects_sub_millisecond_micros() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        let row_key = uaid.simple().to_string();
        let _ = client.remove_user(&uaid).await;

        let mut req = client.mutate_row_request(&row_key);
        req.mutations = vec![bigtable::Mutation {
            mutation: Some(bigtable::mutation::Mutation::SetCell(
                bigtable::mutation::SetCell {
                    family_name: ROUTER_FAMILY.to_owned(),
                    column_qualifier: b"granularity_probe".to_vec(),
                    timestamp_micros: 1,
                    value: b"x".to_vec(),
                },
            )),
        }];

        assert!(
            client.mutate_row(req).await.is_err(),
            "expected granularity mismatch for timestamp_micros=1"
        );

        let _ = client.remove_user(&uaid).await;
    }

    /// run a gauntlet of testing. These are a bit linear because they need
    /// to run in sequence.
    #[actix_rt::test]
    async fn run_gauntlet() -> DbResult<()> {
        let client = new_client()?;

        let connected_at = ms_since_epoch();

        let uaid = Uuid::parse_str(TEST_USER).unwrap();
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
        client.add_user(&test_user).await?;
        let fetched = client.get_user(&uaid).await?;
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.router_type, "webpush".to_owned());

        // Simulate a connected_at occuring before the following writes
        let connected_at = ms_since_epoch();

        // can we add channels?
        client.add_channel(&uaid, &chid).await?;
        let channels = client.get_channels(&uaid).await?;
        assert!(channels.contains(&chid));

        // can we add lots of channels?
        let mut new_channels: HashSet<Uuid> = HashSet::new();
        new_channels.insert(chid);
        for _ in 1..10 {
            new_channels.insert(uuid::Uuid::new_v4());
        }
        let chid_to_remove = uuid::Uuid::new_v4();
        new_channels.insert(chid_to_remove);
        client.add_channels(&uaid, new_channels.clone()).await?;
        let channels = client.get_channels(&uaid).await?;
        assert_eq!(channels, new_channels);

        // can we remove a channel?
        assert!(client.remove_channel(&uaid, &chid_to_remove).await?);
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
        let result = client.update_user(&mut updated).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_ne!(
            fetched2.connected_at,
            client.get_user(&uaid).await?.unwrap().connected_at
        );

        // can we increment the storage for the user?
        client
            .increment_storage(
                &fetched.uaid,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .await?;

        let test_data = "An_encrypted_pile_of_crap".to_owned();
        let timestamp = now();
        let sort_key = now();
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
        let res = client.save_message(&uaid, test_notification.clone()).await;
        assert!(res.is_ok());

        let mut fetched = client.fetch_timestamp_messages(&uaid, None, 999).await?;
        assert_ne!(fetched.messages.len(), 0);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab all 1 of the messages that were submmited within the past 10 seconds.
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(timestamp - 10), 999)
            .await?;
        assert_ne!(fetched.messages.len(), 0);

        // Try grabbing a message for 10 seconds from now.
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(timestamp + 10), 999)
            .await?;
        assert_eq!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(
            client
                .remove_message(&uaid, &test_notification.chidmessageid())
                .await
                .is_ok()
        );

        assert!(client.remove_channel(&uaid, &chid).await.is_ok());

        // Now, can we do all that with topic messages
        client.add_channel(&uaid, &topic_chid).await?;
        let test_data = "An_encrypted_pile_of_crap_with_a_topic".to_owned();
        let timestamp = now();
        let sort_key = now();
        // Can we store a message?
        let test_notification = crate::db::Notification {
            channel_id: topic_chid,
            version: "test".to_owned(),
            ttl: 300,
            topic: Some("topic".to_owned()),
            timestamp,
            data: Some(test_data.clone()),
            sortkey_timestamp: Some(sort_key),
            ..Default::default()
        };
        assert!(
            client
                .save_message(&uaid, test_notification.clone())
                .await
                .is_ok()
        );

        let mut fetched = client.fetch_topic_messages(&uaid, 999).await?;
        assert_ne!(fetched.messages.len(), 0);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab the message that was submmited.
        let fetched = client.fetch_topic_messages(&uaid, 999).await?;
        assert_ne!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(
            client
                .remove_message(&uaid, &test_notification.chidmessageid())
                .await
                .is_ok()
        );

        assert!(client.remove_channel(&uaid, &topic_chid).await.is_ok());

        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        let fetched = client.get_user(&uaid).await?.unwrap();
        assert!(
            client
                .remove_node_id(&uaid, &node_id, connected_at, &fetched.version)
                .await
                .is_ok()
        );
        // did we remove it?
        let fetched = client.get_user(&uaid).await?.unwrap();
        assert_eq!(fetched.node_id, None);

        assert!(client.remove_user(&uaid).await.is_ok());

        assert!(client.get_user(&uaid).await?.is_none());

        Ok(())
    }

    #[actix_rt::test]
    async fn read_cells_family_id() -> DbResult<()> {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        client.remove_user(&uaid).await.unwrap();

        let qualifier = "foo".to_owned();

        let row_key = uaid.simple().to_string();
        let mut row = Row::new(row_key.clone());
        row.cells.insert(
            ROUTER_FAMILY.to_owned(),
            vec![cell::Cell {
                qualifier: qualifier.to_owned(),
                value: "bar".as_bytes().to_vec(),
                ..Default::default()
            }],
        );
        client.write_row(row).await.unwrap();
        let req = client.read_row_request(&row_key);
        let Some(row) = client.read_row(req).await.unwrap() else {
            panic!("Expected row");
        };
        assert_eq!(row.cells.len(), 1);
        assert_eq!(row.cells.keys().next().unwrap(), qualifier.as_str());
        client.remove_user(&uaid).await
    }

    #[actix_rt::test]
    async fn add_user_existing() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        let user = User {
            uaid,
            ..Default::default()
        };
        client.remove_user(&uaid).await.unwrap();

        client.add_user(&user).await.unwrap();
        let err = client.add_user(&user).await.unwrap_err();
        assert!(matches!(err, DbError::Conditional));
    }

    #[actix_rt::test]
    async fn version_check() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        let user = User {
            uaid,
            ..Default::default()
        };
        client.remove_user(&uaid).await.unwrap();

        client.add_user(&user).await.unwrap();
        let mut user = client.get_user(&uaid).await.unwrap().unwrap();
        assert!(client.update_user(&mut user.clone()).await.unwrap());

        let fetched = client.get_user(&uaid).await.unwrap().unwrap();
        assert_ne!(user.version, fetched.version);
        // should now fail w/ a stale version
        assert!(!client.update_user(&mut user).await.unwrap());

        client.remove_user(&uaid).await.unwrap();
    }

    #[actix_rt::test]
    async fn lingering_chid_record() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let user = User {
            uaid,
            ..Default::default()
        };
        client.remove_user(&uaid).await.unwrap();

        // add_channel doesn't check for the existence of a user
        client.add_channel(&uaid, &chid).await.unwrap();

        // w/ chid records in the router row, get_user should treat
        // this as the user not existing
        assert!(client.get_user(&uaid).await.unwrap().is_none());

        client.add_user(&user).await.unwrap();
        // get_user should have also cleaned up the chids
        assert!(client.get_channels(&uaid).await.unwrap().is_empty());

        client.remove_user(&uaid).await.unwrap();
    }

    #[actix_rt::test]
    async fn lingering_current_timestamp() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        client.remove_user(&uaid).await.unwrap();

        client
            .increment_storage(&uaid, crate::util::sec_since_epoch())
            .await
            .unwrap();
        assert!(client.get_user(&uaid).await.unwrap().is_none());

        client.remove_user(&uaid).await.unwrap();
    }

    #[actix_rt::test]
    async fn lingering_chid_w_version_record() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        client.remove_user(&uaid).await.unwrap();

        client.add_channel(&uaid, &chid).await.unwrap();
        assert!(client.remove_channel(&uaid, &chid).await.unwrap());
        assert!(client.get_user(&uaid).await.unwrap().is_none());

        client.remove_user(&uaid).await.unwrap();
    }

    #[actix_rt::test]
    async fn channel_and_current_timestamp_ttl_updates() {
        let client = new_client().unwrap();
        let uaid = gen_test_uaid();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        client.remove_user(&uaid).await.unwrap();

        // Setup a user with some channels and a current_timestamp
        let user = User {
            uaid,
            ..Default::default()
        };
        client.add_user(&user).await.unwrap();

        client.add_channel(&uaid, &chid).await.unwrap();
        client
            .add_channel(&uaid, &uuid::Uuid::new_v4())
            .await
            .unwrap();

        client
            .increment_storage(
                &uaid,
                SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            )
            .await
            .unwrap();

        let req = client.read_row_request(&uaid.as_simple().to_string());
        let Some(mut row) = client.read_row(req).await.unwrap() else {
            panic!("Expected row");
        };

        // Ensure the initial cell expiry (timestamp) of all the cells
        // in the row has been updated
        let ca_expiry = row.take_required_cell("connected_at").unwrap().timestamp;
        for mut cells in row.cells.into_values() {
            let Some(cell) = cells.pop() else {
                continue;
            };
            assert!(
                cell.timestamp >= ca_expiry,
                "{} cell timestamp should >= connected_at's",
                cell.qualifier
            );
        }

        let mut user = client.get_user(&uaid).await.unwrap().unwrap();

        // Quick nap to make sure that the ca_expiry values are different.
        tokio::time::sleep(Duration::from_secs_f32(0.2)).await;
        client.update_user(&mut user).await.unwrap();

        // Ensure update_user updated the expiry (timestamp) of every cell in the row
        let req = client.read_row_request(&uaid.as_simple().to_string());
        let Some(mut row) = client.read_row(req).await.unwrap() else {
            panic!("Expected row");
        };

        let ca_expiry2 = row.take_required_cell("connected_at").unwrap().timestamp;

        assert!(ca_expiry2 > ca_expiry);

        for mut cells in row.cells.into_values() {
            let Some(cell) = cells.pop() else {
                continue;
            };
            assert!(
                cell.timestamp >= ca_expiry2,
                "{} cell timestamp expiry should exceed connected_at's",
                cell.qualifier
            );
        }

        client.remove_user(&uaid).await.unwrap();
    }
}
