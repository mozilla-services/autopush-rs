use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use again::RetryPolicy;
use async_trait::async_trait;
use cadence::{CountedExt, StatsdClient};
use futures_util::StreamExt;
use google_cloud_rust_raw::bigtable::admin::v2::bigtable_table_admin::DropRowRangeRequest;
use google_cloud_rust_raw::bigtable::admin::v2::bigtable_table_admin_grpc::BigtableTableAdminClient;
use google_cloud_rust_raw::bigtable::v2::bigtable::ReadRowsRequest;
use google_cloud_rust_raw::bigtable::v2::bigtable_grpc::BigtableClient;
use google_cloud_rust_raw::bigtable::v2::data::{RowFilter, RowFilter_Chain};
use google_cloud_rust_raw::bigtable::v2::{bigtable, data};
use grpcio::{Channel, Metadata, RpcStatus, RpcStatusCode};
use protobuf::RepeatedField;
use serde_json::{from_str, json};
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, NotificationRecord, User, MAX_CHANNEL_TTL, MAX_ROUTER_TTL,
    USER_RECORD_VERSION,
};

pub use self::metadata::MetadataBuilder;
use self::row::Row;
use super::pool::BigTablePool;
use super::BigTableDbSettings;

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

pub(crate) const RETRY_COUNT: usize = 5;

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
/// Wrapper for the BigTable connection
pub struct BigTableClientImpl {
    pub(crate) settings: BigTableDbSettings,
    /// Metrics client
    metrics: Arc<StatsdClient>,
    /// Connection Channel (used for alternate calls)
    pool: BigTablePool,
    metadata: Metadata,
    admin_metadata: Metadata,
}

/// Return a a RowFilter matching the GC policy of the router Column Family
fn router_gc_policy_filter() -> data::RowFilter {
    let mut latest_cell_filter = data::RowFilter::default();
    latest_cell_filter.set_cells_per_column_limit_filter(1);
    latest_cell_filter
}

/// Return a chain of RowFilters matching the GC policy of the message Column
/// Families
fn message_gc_policy_filter() -> Result<Vec<data::RowFilter>, error::BigTableError> {
    let mut timestamp_filter = data::RowFilter::default();
    let bt_now: i64 = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(error::BigTableError::WriteTime)?
        .as_millis() as i64;
    let mut range_filter = data::TimestampRange::default();
    range_filter.set_start_timestamp_micros(bt_now * 1000);
    timestamp_filter.set_timestamp_range_filter(range_filter);

    Ok(vec![router_gc_policy_filter(), timestamp_filter])
}

/// Return a Column family regex RowFilter
fn family_filter(regex: String) -> data::RowFilter {
    let mut filter = data::RowFilter::default();
    filter.set_family_name_regex_filter(regex);
    filter
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
fn version_filter(version: &Uuid) -> Vec<data::RowFilter> {
    let mut cq_filter = data::RowFilter::default();
    cq_filter.set_column_qualifier_regex_filter("^version$".as_bytes().to_vec());

    let mut value_filter = data::RowFilter::default();
    value_filter.set_value_regex_filter(escape_bytes(version.as_bytes()));

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
fn filter_chain(filters: impl Into<RepeatedField<RowFilter>>) -> RowFilter {
    let mut chain = RowFilter_Chain::default();
    chain.set_filters(filters.into());
    let mut filter = RowFilter::default();
    filter.set_chain(chain);
    filter
}

/// Return a ReadRowsRequest against table for a given row key
fn read_row_request(
    table_name: &str,
    app_profile_id: &str,
    row_key: &str,
) -> bigtable::ReadRowsRequest {
    let mut req = bigtable::ReadRowsRequest::default();
    req.set_table_name(table_name.to_owned());
    req.set_app_profile_id(app_profile_id.to_owned());

    let mut row_keys = RepeatedField::default();
    row_keys.push(row_key.as_bytes().to_vec());
    let mut row_set = data::RowSet::default();
    row_set.set_row_keys(row_keys);
    req.set_rows(row_set);

    req
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

pub fn retry_policy(max: usize) -> RetryPolicy {
    RetryPolicy::default()
        .with_max_retries(max)
        .with_jitter(true)
}

fn retryable_internal_error(status: &RpcStatus) -> bool {
    match status.code() {
        RpcStatusCode::UNKNOWN => {
            "error occurred when fetching oauth2 token." == status.message().to_ascii_lowercase()
        }
        RpcStatusCode::INTERNAL => [
            "rst_stream",
            "rst stream",
            "received unexpected eos on data frame from server",
        ]
        .contains(&status.message().to_lowercase().as_str()),
        RpcStatusCode::UNAVAILABLE | RpcStatusCode::DEADLINE_EXCEEDED => true,
        _ => false,
    }
}

pub fn metric(metrics: &Arc<StatsdClient>, err_type: &str, code: Option<&str>) {
    let mut metric = metrics
        .incr_with_tags("database.retry")
        .with_tag("error", err_type)
        .with_tag("type", "bigtable");
    if let Some(code) = code {
        metric = metric.with_tag("code", code);
    }
    metric.send();
}

pub fn retryable_error(metrics: Arc<StatsdClient>) -> impl Fn(&grpcio::Error) -> bool {
    move |err| {
        debug!("🉑 Checking error...{err}");
        match err {
            grpcio::Error::RpcFailure(status) => {
                info!("GRPC Failure :{:?}", status);
                let retry = retryable_internal_error(status);
                if retry {
                    metric(&metrics, "RpcFailure", Some(&status.code().to_string()));
                }
                retry
            }
            grpcio::Error::BindFail(_) => {
                metric(&metrics, "BindFail", None);
                true
            }
            // The parameter here is a [grpcio_sys::grpc_call_error] enum
            // Not all of these are retryable.
            grpcio::Error::CallFailure(grpc_call_status) => {
                let retry = grpc_call_status == &grpcio_sys::grpc_call_error::GRPC_CALL_ERROR;
                if retry {
                    metric(
                        &metrics,
                        "CallFailure",
                        Some(&format!("{:?}", grpc_call_status)),
                    );
                }
                retry
            }
            _ => false,
        }
    }
}

/// Determine if a router record is "incomplete" (doesn't include [User]
/// columns):
///
/// They can be incomplete for a couple reasons:
///
/// 1) A migration code bug caused a few incomplete migrations where
/// `add_channels` and `increment_storage` calls occurred when the migration's
/// initial `add_user` was never completed:
/// https://github.com/mozilla-services/autopush-rs/pull/640
///
/// 2) When router TTLs are eventually enabled: `add_channel` and
/// `increment_storage` can write cells with later expiry times than the other
/// router cells
fn is_incomplete_router_record(cells: &HashMap<String, Vec<cell::Cell>>) -> bool {
    cells
        .keys()
        .all(|k| ["current_timestamp", "version"].contains(&k.as_str()) || k.starts_with("chid:"))
}

fn call_opts(metadata: Metadata) -> ::grpcio::CallOption {
    ::grpcio::CallOption::default().headers(metadata)
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
        // let env = Arc::new(EnvBuilder::new().build());
        debug!("🏊 BT Pool new");
        let db_settings = BigTableDbSettings::try_from(settings.db_settings.as_ref())?;
        info!("🉑 {:#?}", db_settings);
        let pool = BigTablePool::new(settings, &metrics)?;

        // create the metadata header blocks required by Google for accessing GRPC resources.
        let metadata = db_settings.metadata()?;
        let admin_metadata = db_settings.admin_metadata()?;
        Ok(Self {
            settings: db_settings,
            metrics,
            metadata,
            admin_metadata,
            pool,
        })
    }

    /// Spawn a task to periodically evict idle connections
    pub fn spawn_sweeper(&self, interval: Duration) {
        self.pool.spawn_sweeper(interval);
    }

    /// Return a ReadRowsRequest for a given row key
    fn read_row_request(&self, row_key: &str) -> bigtable::ReadRowsRequest {
        read_row_request(
            &self.settings.table_name,
            &self.settings.profile_id,
            row_key,
        )
    }

    /// Return a MutateRowRequest for a given row key
    fn mutate_row_request(&self, row_key: &str) -> bigtable::MutateRowRequest {
        let mut req = bigtable::MutateRowRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_app_profile_id(self.settings.profile_id.clone());
        req.set_row_key(row_key.as_bytes().to_vec());
        req
    }

    /// Return a CheckAndMutateRowRequest for a given row key
    fn check_and_mutate_row_request(&self, row_key: &str) -> bigtable::CheckAndMutateRowRequest {
        let mut req = bigtable::CheckAndMutateRowRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_app_profile_id(self.settings.profile_id.clone());
        req.set_row_key(row_key.as_bytes().to_vec());
        req
    }

    /// Read a given row from the row key.
    async fn mutate_row(
        &self,
        req: bigtable::MutateRowRequest,
    ) -> Result<(), error::BigTableError> {
        let bigtable = self.pool.get().await?;
        retry_policy(self.settings.retry_count)
            .retry_if(
                || async {
                    bigtable
                        .conn
                        .mutate_row_opt(&req, call_opts(self.metadata.clone()))
                },
                retryable_error(self.metrics.clone()),
            )
            .await
            .map_err(error::BigTableError::Write)?;
        Ok(())
    }

    /// Perform a MutateRowsRequest
    #[allow(unused)]
    async fn mutate_rows(
        &self,
        req: bigtable::MutateRowsRequest,
    ) -> Result<(), error::BigTableError> {
        let bigtable = self.pool.get().await?;
        // ClientSStreamReceiver will cancel an operation if it's dropped before it's done.
        let resp = retry_policy(self.settings.retry_count)
            .retry_if(
                || async {
                    bigtable
                        .conn
                        .mutate_rows_opt(&req, call_opts(self.metadata.clone()))
                },
                retryable_error(self.metrics.clone()),
            )
            .await
            .map_err(error::BigTableError::Write)?;

        // Scan the returned stream looking for errors.
        // As I understand, the returned stream contains chunked MutateRowsResponse structs. Each
        // struct contains the result of the row mutation, and contains a `status` (non-zero on error)
        // and an optional message string (empty if none).
        // The structure also contains an overall `status` but that does not appear to be exposed.
        // Status codes are defined at https://grpc.github.io/grpc/core/md_doc_statuscodes.html
        let mut stream = Box::pin(resp);
        let mut cnt = 0;
        loop {
            let (result, remainder) = stream.into_future().await;
            if let Some(result) = result {
                debug!("🎏 Result block: {}", cnt);
                match result {
                    Ok(r) => {
                        for e in r.get_entries() {
                            if e.has_status() {
                                let status = e.get_status();
                                // See status code definitions: https://grpc.github.io/grpc/core/md_doc_statuscodes.html
                                let code = error::MutateRowStatus::from(status.get_code());
                                if !code.is_ok() {
                                    return Err(error::BigTableError::Status(
                                        code,
                                        status.get_message().to_owned(),
                                    ));
                                }
                                debug!("🎏 Response: {} OK", e.index);
                            }
                        }
                    }
                    Err(e) => return Err(error::BigTableError::Write(e)),
                };
                cnt += 1;
            } else {
                debug!("🎏 Done!");
                break;
            }
            stream = remainder;
        }

        Ok(())
    }

    /// Read one row for the [ReadRowsRequest] (assuming only a single row was requested).
    async fn read_row(
        &self,
        req: bigtable::ReadRowsRequest,
    ) -> Result<Option<row::Row>, error::BigTableError> {
        let mut rows = self.read_rows(req).await?;
        Ok(rows.pop_first().map(|(_, v)| v))
    }

    /// Take a big table ReadRowsRequest (containing the keys and filters) and return a set of row data indexed by row key.
    ///
    ///
    async fn read_rows(
        &self,
        req: ReadRowsRequest,
    ) -> Result<BTreeMap<RowKey, row::Row>, error::BigTableError> {
        let bigtable = self.pool.get().await?;
        let resp = retry_policy(self.settings.retry_count)
            .retry_if(
                || async {
                    bigtable
                        .conn
                        .read_rows_opt(&req, call_opts(self.metadata.clone()))
                },
                retryable_error(self.metrics.clone()),
            )
            .await
            .map_err(error::BigTableError::Read)?;
        merge::RowMerger::process_chunks(resp).await
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
        let mutations = self.get_mutations(row.cells)?;
        req.set_mutations(mutations);
        self.mutate_row(req).await?;
        Ok(())
    }

    /// Compile the list of mutations for this row.
    fn get_mutations(
        &self,
        cells: HashMap<FamilyId, Vec<crate::db::bigtable::bigtable_client::cell::Cell>>,
    ) -> Result<protobuf::RepeatedField<data::Mutation>, error::BigTableError> {
        let mut mutations = protobuf::RepeatedField::default();
        for (family_id, cells) in cells {
            for cell in cells {
                let mut mutation = data::Mutation::default();
                let mut set_cell = data::Mutation_SetCell::default();
                let timestamp = cell
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(error::BigTableError::WriteTime)?;
                set_cell.family_name = family_id.clone();
                set_cell.set_column_qualifier(cell.qualifier.clone().into_bytes());
                set_cell.set_value(cell.value);
                // Yes, this is passing milli bounded time as a micro. Otherwise I get
                // a `Timestamp granularity mismatch` error
                set_cell.set_timestamp_micros((timestamp.as_millis() * 1000) as i64);
                debug!("🉑 expiring in {:?}", timestamp.as_millis());
                mutation.set_set_cell(set_cell);
                mutations.push(mutation);
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
        filter: RowFilter,
        state: bool,
    ) -> Result<bool, error::BigTableError> {
        let mut req = self.check_and_mutate_row_request(&row.row_key);
        let mutations = self.get_mutations(row.cells)?;
        req.set_predicate_filter(filter);
        if state {
            req.set_true_mutations(mutations);
        } else {
            req.set_false_mutations(mutations);
        }
        self.check_and_mutate(req).await
    }

    async fn check_and_mutate(
        &self,
        req: bigtable::CheckAndMutateRowRequest,
    ) -> Result<bool, error::BigTableError> {
        let bigtable = self.pool.get().await?;
        let resp = retry_policy(self.settings.retry_count)
            .retry_if(
                || async {
                    // Note: check_and_mutate_row_async may return before the row
                    // is written, which can cause race conditions for reads
                    bigtable
                        .conn
                        .check_and_mutate_row_opt(&req, call_opts(self.metadata.clone()))
                },
                retryable_error(self.metrics.clone()),
            )
            .await
            .map_err(error::BigTableError::Write)?;
        debug!("🉑 Predicate Matched: {}", &resp.get_predicate_matched(),);
        Ok(resp.get_predicate_matched())
    }

    fn get_delete_mutations(
        &self,
        family: &str,
        column_names: &[&str],
        time_range: Option<&data::TimestampRange>,
    ) -> Result<protobuf::RepeatedField<data::Mutation>, error::BigTableError> {
        let mut mutations = protobuf::RepeatedField::default();
        for column in column_names {
            let mut mutation = data::Mutation::default();
            // Mutation_DeleteFromRow -- Delete all cells for a given row.
            // Mutation_DeleteFromFamily -- Delete all cells from a family for a given row.
            // Mutation_DeleteFromColumn -- Delete all cells from a column name for a given row, restricted by timestamp range.
            let mut del_cell = data::Mutation_DeleteFromColumn::default();
            del_cell.set_family_name(family.to_owned());
            del_cell.set_column_qualifier(column.as_bytes().to_vec());
            if let Some(range) = time_range {
                del_cell.set_time_range(range.clone());
            }
            mutation.set_delete_from_column(del_cell);
            mutations.push(mutation);
        }
        Ok(mutations)
    }

    #[allow(unused)]
    /// Delete all cell data from the specified columns with the optional time range.
    #[allow(unused)]
    async fn delete_cells(
        &self,
        row_key: &str,
        family: &str,
        column_names: &[&str],
        time_range: Option<&data::TimestampRange>,
    ) -> Result<(), error::BigTableError> {
        let mut req = self.mutate_row_request(row_key);
        req.set_mutations(self.get_delete_mutations(family, column_names, time_range)?);
        self.mutate_row(req).await
    }

    /// Delete all the cells for the given row. NOTE: This will drop the row.
    async fn delete_row(&self, row_key: &str) -> Result<(), error::BigTableError> {
        let mut req = self.mutate_row_request(row_key);
        let mut mutations = protobuf::RepeatedField::default();
        let mut mutation = data::Mutation::default();
        mutation.set_delete_from_row(data::Mutation_DeleteFromRow::default());
        mutations.push(mutation);
        req.set_mutations(mutations);
        self.mutate_row(req).await
    }

    /// This uses the admin interface to drop row ranges.
    /// This will drop ALL data associated with these rows.
    /// Note that deletion may take up to a week to occur.
    /// see https://cloud.google.com/php/docs/reference/cloud-bigtable/latest/Admin.V2.DropRowRangeRequest
    #[allow(unused)]
    async fn delete_rows(&self, row_key: &str) -> Result<bool, error::BigTableError> {
        let admin = BigtableTableAdminClient::new(self.pool.get_channel()?);
        let mut req = DropRowRangeRequest::new();
        req.set_name(self.settings.table_name.clone());
        req.set_row_key_prefix(row_key.as_bytes().to_vec());

        admin
            .drop_row_range_async_opt(&req, call_opts(self.admin_metadata.clone()))
            .map_err(|e| {
                error!("{:?}", e);
                error::BigTableError::Admin(
                    format!(
                        "Could not send delete command for {}",
                        &self.settings.table_name
                    ),
                    Some(e.to_string()),
                )
            })?
            .await
            .map_err(|e| {
                error!("post await: {:?}", e);
                error::BigTableError::Admin(
                    format!(
                        "Could not delete data from table {}",
                        &self.settings.table_name
                    ),
                    Some(e.to_string()),
                )
            })?;

        Ok(true)
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
        let range_key = NotificationRecord::parse_chidmessageid(chidmessageid).map_err(|e| {
            DbError::Integrity(
                format!("rows_to_notification expected chidmessageid: {e}"),
                None,
            )
        })?;

        let mut notif = Notification {
            channel_id: range_key.channel_id,
            topic: range_key.topic,
            sortkey_timestamp: range_key.sortkey_timestamp,
            version: to_string(row.take_required_cell("version")?.value, "version")?,
            ttl: to_u64(row.take_required_cell("ttl")?.value, "ttl")?,
            timestamp: to_u64(row.take_required_cell("timestamp")?.value, "timestamp")?,
            ..Default::default()
        };

        if let Some(cell) = row.take_cell("data") {
            notif.data = Some(to_string(cell.value, "data")?);
        }
        if let Some(cell) = row.take_cell("headers") {
            notif.headers = Some(
                serde_json::from_str::<HashMap<String, String>>(&to_string(cell.value, "headers")?)
                    .map_err(|e| DbError::Serialization(e.to_string()))?,
            );
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
        let expiry = std::time::SystemTime::now() + Duration::from_secs(MAX_ROUTER_TTL);

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

        row.add_cells(ROUTER_FAMILY, cells);
        row
    }
}

#[derive(Clone)]
pub struct BigtableDb {
    pub(super) conn: BigtableClient,
    pub(super) health_metadata: Metadata,
    table_name: String,
}

impl BigtableDb {
    pub fn new(channel: Channel, health_metadata: &Metadata, table_name: &str) -> Self {
        Self {
            conn: BigtableClient::new(channel),
            health_metadata: health_metadata.clone(),
            table_name: table_name.to_owned(),
        }
    }
    /// Perform a simple connectivity check. This should return no actual results
    /// but should verify that the connection is valid. We use this for the
    /// Recycle check as well, so it has to be fairly low in the implementation
    /// stack.
    ///
    ///
    pub async fn health_check(
        &mut self,
        metrics: &Arc<StatsdClient>,
        app_profile_id: &str,
    ) -> Result<bool, error::BigTableError> {
        // It is recommended that we pick a random key to perform the health check. Selecting
        // a single key for all health checks causes a "hot tablet" to arise. The `PingAndWarm`
        // is intended to be used prior to large updates and is not recommended for use in
        // health checks.
        // This health check is to see if the database is present, the response is not important
        // other than it does not return an error.
        let random_uaid = Uuid::new_v4().simple().to_string();
        let mut req = read_row_request(&self.table_name, app_profile_id, &random_uaid);
        let mut filter = data::RowFilter::default();
        filter.set_block_all_filter(true);
        req.set_filter(filter);
        let _r = retry_policy(RETRY_COUNT)
            .retry_if(
                || async {
                    self.conn
                        .read_rows_opt(&req, call_opts(self.health_metadata.clone()))
                },
                retryable_error(metrics.clone()),
            )
            .await
            .map_err(error::BigTableError::Read)?;

        debug!("🉑 health check");
        Ok(true)
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
        let mut row_key_filter = RowFilter::default();
        row_key_filter.set_row_key_regex_filter(format!("^{}$", row.row_key).into_bytes());
        let filter = filter_chain(vec![router_gc_policy_filter(), row_key_filter]);

        if self.check_and_mutate_row(row, filter, false).await? {
            return Err(DbError::Conditional);
        }
        Ok(())
    }

    /// BigTable doesn't really have the concept of an "update". You simply write the data and
    /// the individual cells create a new version. Depending on the garbage collection rules for
    /// the family, these can either persist or be automatically deleted.
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
        req.set_filter(filter_chain(filters));
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
                    .incr_with_tags("database.drop_user")
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
        let expiry = std::time::SystemTime::now() + Duration::from_secs(MAX_CHANNEL_TTL);

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
        // Note: updating the version column isn't necessary here because this
        // write only adds a new (or updates an existing) column with a 0 byte
        // value
        row.add_cells(ROUTER_FAMILY, cells);

        self.write_row(row).await?;
        Ok(())
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let row_key = uaid.simple().to_string();
        let mut req = self.read_row_request(&row_key);

        let mut cq_filter = data::RowFilter::default();
        cq_filter.set_column_qualifier_regex_filter("^chid:.*$".as_bytes().to_vec());
        req.set_filter(filter_chain(vec![
            router_gc_policy_filter(),
            family_filter(format!("^{ROUTER_FAMILY}$")),
            cq_filter,
        ]));

        let mut result = HashSet::new();
        if let Some(record) = self.read_row(req).await? {
            for mut cells in record.cells.into_values() {
                let Some(cell) = cells.pop() else {
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
        }

        Ok(result)
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
        let expiry = std::time::SystemTime::now() + Duration::from_secs(MAX_ROUTER_TTL);
        row.cells
            .insert(ROUTER_FAMILY.to_owned(), vec![new_version_cell(expiry)]);
        mutations.extend(self.get_mutations(row.cells)?);

        // check if the channel existed/was actually removed
        let mut cq_filter = data::RowFilter::default();
        cq_filter.set_column_qualifier_regex_filter(format!("^{column}$").into_bytes());
        req.set_predicate_filter(filter_chain(vec![router_gc_policy_filter(), cq_filter]));
        req.set_true_mutations(mutations);

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
        let Some(ref version) = version else {
            return Err(DbError::General("Expected a user version field".to_owned()));
        };

        let mut req = self.check_and_mutate_row_request(&row_key);

        let mut filters = vec![router_gc_policy_filter()];
        filters.extend(version_filter(version));
        req.set_predicate_filter(filter_chain(filters));
        req.set_true_mutations(self.get_delete_mutations(ROUTER_FAMILY, &["node_id"], None)?);

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
        if let Some(headers) = message.headers {
            if !headers.is_empty() {
                cells.push(cell::Cell {
                    qualifier: "headers".to_owned(),
                    value: json!(headers).to_string().into_bytes(),
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
            .incr_with_tags("notification.message.stored")
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
        let mut row = Row::new(row_key);
        let expiry = std::time::SystemTime::now() + Duration::from_secs(MAX_ROUTER_TTL);
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
            .incr_with_tags("notification.message.deleted")
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
        let mut req = ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_app_profile_id(self.settings.profile_id.clone());

        let start_key = format!("{}#01:", uaid.simple());
        let end_key = format!("{}#02:", uaid.simple());
        let mut rows = data::RowSet::default();
        let mut row_range = data::RowRange::default();
        row_range.set_start_key_open(start_key.into_bytes());
        row_range.set_end_key_open(end_key.into_bytes());
        let mut row_ranges = RepeatedField::default();
        row_ranges.push(row_range);
        rows.set_row_ranges(row_ranges);
        req.set_rows(rows);

        let mut filters = message_gc_policy_filter()?;
        filters.push(family_filter(format!("^{MESSAGE_TOPIC_FAMILY}$")));

        req.set_filter(filter_chain(filters));
        if limit > 0 {
            trace!("🉑 Setting limit to {limit}");
            req.set_rows_limit(limit as i64);
        }
        let rows = self.read_rows(req).await?;
        debug!(
            "🉑 Fetch Topic Messages. Found {} row(s) of {}",
            rows.len(),
            limit
        );

        let messages = self.rows_to_notifications(rows)?;
        // Note: Bigtable always returns a timestamp of None here whereas
        // DynamoDB returns the `current_timestamp` read from its meta
        // record. Under Bigtable `current_timestamp` is instead initially read
        // from [get_user]
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
        let mut req = ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_app_profile_id(self.settings.profile_id.clone());

        let mut rows = data::RowSet::default();
        let mut row_range = data::RowRange::default();

        let start_key = if let Some(ts) = timestamp {
            // Fetch everything after the last message with timestamp: the "z"
            // moves past the last message's channel_id's 1st hex digit
            format!("{}#02:{}z", uaid.simple(), ts)
        } else {
            format!("{}#02:", uaid.simple())
        };
        let end_key = format!("{}#03:", uaid.simple());
        row_range.set_start_key_open(start_key.into_bytes());
        row_range.set_end_key_open(end_key.into_bytes());

        let mut row_ranges = RepeatedField::default();
        row_ranges.push(row_range);
        rows.set_row_ranges(row_ranges);
        req.set_rows(rows);

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

        req.set_filter(filter_chain(filters));
        if limit > 0 {
            req.set_rows_limit(limit as i64);
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
        Ok(self
            .pool
            .get()
            .await?
            .health_check(&self.metrics.clone(), &self.settings.profile_id)
            .await?)
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

    /// BigTable does not support message table rotation
    fn rotating_message_table(&self) -> Option<&str> {
        None
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
            last_connect: Some(connected_at),
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
        assert!(client
            .remove_message(&uaid, &test_notification.chidmessageid())
            .await
            .is_ok());

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
        assert!(client
            .save_message(&uaid, test_notification.clone())
            .await
            .is_ok());

        let mut fetched = client.fetch_topic_messages(&uaid, 999).await?;
        assert_ne!(fetched.messages.len(), 0);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab the message that was submmited.
        let fetched = client.fetch_topic_messages(&uaid, 999).await?;
        assert_ne!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(client
            .remove_message(&uaid, &test_notification.chidmessageid())
            .await
            .is_ok());

        assert!(client.remove_channel(&uaid, &topic_chid).await.is_ok());

        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        let fetched = client.get_user(&uaid).await?.unwrap();
        assert!(client
            .remove_node_id(&uaid, &node_id, connected_at, &fetched.version)
            .await
            .is_ok());
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
            .increment_storage(&uaid, ms_since_epoch())
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
}
