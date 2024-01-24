use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cadence::StatsdClient;
use futures_util::StreamExt;
use google_cloud_rust_raw::bigtable::admin::v2::bigtable_table_admin::DropRowRangeRequest;
use google_cloud_rust_raw::bigtable::admin::v2::bigtable_table_admin_grpc::BigtableTableAdminClient;
use google_cloud_rust_raw::bigtable::v2::bigtable::ReadRowsRequest;
use google_cloud_rust_raw::bigtable::v2::bigtable_grpc::BigtableClient;
use google_cloud_rust_raw::bigtable::v2::data::{
    RowFilter, RowFilter_Chain, RowFilter_Condition, ValueRange,
};
use google_cloud_rust_raw::bigtable::v2::{bigtable, data};
use grpcio::Channel;
use protobuf::RepeatedField;
use serde_json::{from_str, json};
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, User,
};

use self::row::Row;
use super::pool::BigTablePool;
use super::BigTableDbSettings;

pub mod cell;
pub mod error;
pub(crate) mod merge;
pub mod row;

// these are normally Vec<u8>
pub type RowKey = String;
pub type Qualifier = String;
// This must be a String.
pub type FamilyId = String;

const ROUTER_FAMILY: &str = "router";
const MESSAGE_FAMILY: &str = "message"; // The default family for messages
const MESSAGE_TOPIC_FAMILY: &str = "message_topic";

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
    _metrics: Arc<StatsdClient>,
    /// Connection Channel (used for alternate calls)
    pool: BigTablePool,
}

fn timestamp_filter() -> Result<data::RowFilter, error::BigTableError> {
    let mut timestamp_filter = data::RowFilter::default();
    let bt_now: i64 = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(error::BigTableError::WriteTime)?
        .as_millis() as i64;
    let mut range_filter = data::TimestampRange::default();
    range_filter.set_start_timestamp_micros(bt_now * 1000);
    timestamp_filter.set_timestamp_range_filter(range_filter);

    Ok(timestamp_filter)
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
        debug!("🉑 {:#?}", db_settings);
        let pool = BigTablePool::new(settings, &metrics)?;
        Ok(Self {
            settings: db_settings,
            _metrics: metrics,
            pool,
        })
    }

    /// Read a given row from the row key.
    async fn read_row(
        &self,
        row_key: &str,
        timestamp_filter: Option<u64>,
    ) -> Result<Option<row::Row>, error::BigTableError> {
        debug!("🉑 Row key: {}", row_key);

        let mut row_keys = RepeatedField::default();
        row_keys.push(row_key.as_bytes().to_vec());

        let mut row_set = data::RowSet::default();
        row_set.set_row_keys(row_keys);

        let mut req = bigtable::ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_rows(row_set);

        let mut rows = self.read_rows(req, timestamp_filter, None).await?;
        Ok(rows.remove(row_key))
    }

    /// Take a big table ReadRowsRequest (containing the keys and filters) and return a set of row data indexed by row key.
    ///
    ///
    async fn read_rows(
        &self,
        req: ReadRowsRequest,
        sortkey_filter: Option<u64>,
        limit: Option<usize>,
    ) -> Result<BTreeMap<RowKey, row::Row>, error::BigTableError> {
        let bigtable = self.pool.get().await?;
        let resp = bigtable
            .conn
            .read_rows(&req)
            .map_err(error::BigTableError::Read)?;
        merge::RowMerger::process_chunks(resp, sortkey_filter, limit).await
    }

    /// write a given row.
    ///
    /// there's also `.mutate_rows` which I presume allows multiple.
    async fn write_row(&self, row: row::Row) -> Result<(), error::BigTableError> {
        let mut req = bigtable::MutateRowRequest::default();

        // compile the mutations.
        // It's possible to do a lot here, including altering in process
        // mutations, clearing them, etc. It's all up for grabs until we commit
        // below. For now, let's just presume a write and be done.
        req.set_table_name(self.settings.table_name.clone());
        req.set_row_key(row.row_key.into_bytes());
        let mutations = self.get_mutations(row.cells)?;
        req.set_mutations(mutations);

        // Do the actual commit.
        let bigtable = self.pool.get().await?;
        let _resp = bigtable
            .conn
            .mutate_row_async(&req)
            .map_err(error::BigTableError::Write)?
            .await
            .map_err(error::BigTableError::Write)?;
        Ok(())
    }

    /// Compile the list of mutations for this row.
    fn get_mutations(
        &self,
        cells: HashMap<String, Vec<crate::db::bigtable::bigtable_client::cell::Cell>>,
    ) -> Result<protobuf::RepeatedField<data::Mutation>, error::BigTableError> {
        let mut mutations = protobuf::RepeatedField::default();
        for (_family, cells) in cells {
            for cell in cells {
                let mut mutation = data::Mutation::default();
                let mut set_cell = data::Mutation_SetCell::default();
                let timestamp = cell
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(error::BigTableError::WriteTime)?;
                set_cell.family_name = cell.family;
                set_cell.set_column_qualifier(cell.qualifier.into_bytes());
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

    /// Check and write rows that match the associated filter, returning if the filter
    /// matched records (and the update was successful)
    async fn check_and_mutate_row(
        &self,
        row: row::Row,
        filter: RowFilter,
    ) -> Result<bool, error::BigTableError> {
        let mut req = bigtable::CheckAndMutateRowRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_row_key(row.row_key.into_bytes());
        let mutations = self.get_mutations(row.cells)?;
        req.set_predicate_filter(filter);
        req.set_true_mutations(mutations);

        // Do the actual commit.
        let bigtable = self.pool.get().await?;
        let resp = bigtable
            .conn
            .check_and_mutate_row_async(&req)
            .map_err(error::BigTableError::Write)?
            .await
            .map_err(error::BigTableError::Write)?;
        debug!("🉑 Predicate Matched: {}", &resp.get_predicate_matched(),);
        Ok(resp.get_predicate_matched())
    }

    /// Delete all cell data from the specified columns with the optional time range.
    async fn delete_cells(
        &self,
        row_key: &str,
        family: &str,
        column_names: &Vec<&str>,
        time_range: Option<&data::TimestampRange>,
    ) -> Result<(), error::BigTableError> {
        let mut req = bigtable::MutateRowRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        let mut mutations = protobuf::RepeatedField::default();
        req.set_row_key(row_key.to_owned().into_bytes());
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

        req.set_mutations(mutations);

        let bigtable = self.pool.get().await?;
        let _resp = bigtable
            .conn
            .mutate_row_async(&req)
            .map_err(error::BigTableError::Write)?
            .await
            .map_err(error::BigTableError::Write)?;
        Ok(())
    }

    /// Delete all the cells for the given row. NOTE: This will drop the row.
    async fn delete_row(&self, row_key: &str) -> Result<(), error::BigTableError> {
        let mut req = bigtable::MutateRowRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        let mut mutations = protobuf::RepeatedField::default();
        req.set_row_key(row_key.to_owned().into_bytes());
        let mut mutation = data::Mutation::default();
        mutation.set_delete_from_row(data::Mutation_DeleteFromRow::default());
        mutations.push(mutation);
        req.set_mutations(mutations);

        let bigtable = self.pool.get().await?;
        let _resp = bigtable
            .conn
            .mutate_row_async(&req)
            .map_err(error::BigTableError::Write)?
            .await
            .map_err(error::BigTableError::Write)?;
        Ok(())
    }

    /// This uses the admin interface to drop row ranges.
    /// This will drop ALL data associated with these rows.
    /// Note that deletion may take up to a week to occur.
    /// see https://cloud.google.com/php/docs/reference/cloud-bigtable/latest/Admin.V2.DropRowRangeRequest
    async fn delete_rows(&self, row_key: &str) -> Result<bool, error::BigTableError> {
        let admin = BigtableTableAdminClient::new(self.pool.get_channel()?);
        let mut req = DropRowRangeRequest::new();
        req.set_name(self.settings.table_name.clone());
        req.set_row_key_prefix(row_key.as_bytes().to_vec());
        admin
            .drop_row_range_async(&req)
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
        limit: Option<usize>,
    ) -> Result<FetchMessageResponse, crate::db::error::DbError> {
        let mut messages: Vec<Notification> = Vec::new();
        let mut max_timestamp: u64 = 0;

        for (_key, mut row) in rows {
            if let Some(limit) = limit {
                if messages.len() >= limit {
                    break;
                }
            }
            // get the dominant family type for this row.
            if let Some(cell) = row.take_cell("channel_id") {
                let mut notif = Notification {
                    channel_id: Uuid::from_str(&to_string(cell.value, "channel_id")?).map_err(
                        |e| {
                            DbError::Serialization(format!(
                                "Could not deserialize chid to uuid: {:?}",
                                e
                            ))
                        },
                    )?,
                    ..Default::default()
                };
                if let Some(cell) = row.take_cell("version") {
                    notif.version = to_string(cell.value, "version")?;
                }
                if let Some(cell) = row.take_cell("topic") {
                    notif.topic = Some(to_string(cell.value, "topic")?);
                }

                if let Some(cell) = row.take_cell("ttl") {
                    notif.ttl = to_u64(cell.value, "ttl")?;
                }

                if let Some(cell) = row.take_cell("data") {
                    notif.data = Some(to_string(cell.value, "data")?);
                }
                if let Some(cell) = row.take_cell("sortkey_timestamp") {
                    let sk_ts = to_u64(cell.value, "sortkey_timestamp")?;
                    notif.sortkey_timestamp = Some(sk_ts);
                    if sk_ts > max_timestamp {
                        max_timestamp = sk_ts;
                    }
                }
                if let Some(cell) = row.take_cell("timestamp") {
                    notif.timestamp = to_u64(cell.value, "timestamp")?;
                }
                if let Some(cell) = row.take_cell("headers") {
                    notif.headers = Some(
                        serde_json::from_str::<HashMap<String, String>>(&to_string(
                            cell.value, "headers",
                        )?)
                        .map_err(|e| DbError::Serialization(e.to_string()))?,
                    );
                }
                trace!("🚣  adding row: {:?}", &notif);
                messages.push(notif);
            }
        }

        Ok(FetchMessageResponse {
            messages,
            timestamp: if max_timestamp > 0 {
                Some(max_timestamp)
            } else {
                None
            },
        })
    }

    fn user_to_row(&self, user: &User) -> Row {
        let mut row = Row {
            row_key: user.uaid.simple().to_string(),
            ..Default::default()
        };

        let mut cells: Vec<cell::Cell> = vec![
            cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "connected_at".to_owned(),
                value: user.connected_at.to_be_bytes().to_vec(),
                ..Default::default()
            },
            cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "router_type".to_owned(),
                value: user.router_type.clone().into_bytes(),
                ..Default::default()
            },
        ];

        if let Some(router_data) = &user.router_data {
            cells.push(cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "router_data".to_owned(),
                value: json!(router_data).to_string().as_bytes().to_vec(),
                ..Default::default()
            });
        };
        if let Some(current_timestamp) = user.current_timestamp {
            cells.push(cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "current_timestamp".to_owned(),
                value: current_timestamp.to_be_bytes().to_vec(),
                ..Default::default()
            });
        };
        if let Some(node_id) = &user.node_id {
            cells.push(cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "node_id".to_owned(),
                value: node_id.as_bytes().to_vec(),
                ..Default::default()
            });
        };
        if let Some(record_version) = user.record_version {
            cells.push(cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "record_version".to_owned(),
                value: record_version.to_be_bytes().to_vec(),
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
}

impl BigtableDb {
    pub fn new(channel: Channel) -> Self {
        Self {
            conn: BigtableClient::new(channel),
        }
    }

    /// Perform a simple connectivity check. This should return no actual results
    /// but should verify that the connection is valid. We use this for the
    /// Recycle check as well, so it has to be fairly low in the implementation
    /// stack.
    ///
    pub async fn health_check(&mut self, table_name: &str) -> DbResult<bool> {
        // build the associated request.
        let mut req = bigtable::ReadRowsRequest::default();
        req.set_table_name(table_name.to_owned());
        // Create a request that is GRPC valid, but does not point to a valid row.
        let mut row_keys = RepeatedField::default();
        row_keys.push("NOT FOUND".as_bytes().to_vec());
        let mut row_set = data::RowSet::default();
        row_set.set_row_keys(row_keys);
        req.set_rows(row_set);
        let mut filter = data::RowFilter::default();
        filter.set_block_all_filter(true);
        req.set_filter(filter);

        let r = self
            .conn
            .read_rows(&req)
            .map_err(|e| DbError::General(format!("BigTable connectivity error: {:?}", e)))?;

        let (v, _stream) = r.into_future().await;
        // Since this should return no rows (with the row key set to a value that shouldn't exist)
        // the first component of the tuple should be None.
        Ok(v.is_none())
    }
}

#[async_trait]
impl DbClient for BigTableClientImpl {
    /// add user to the database
    async fn add_user(&self, user: &User) -> DbResult<()> {
        let row = self.user_to_row(user);
        trace!("🉑 Adding user");
        self.write_row(row).await.map_err(|e| e.into())
    }

    /// BigTable doesn't really have the concept of an "update". You simply write the data and
    /// the individual cells create a new version. Depending on the garbage collection rules for
    /// the family, these can either persist or be automatically deleted.
    async fn update_user(&self, user: &User) -> DbResult<bool> {
        // `update_user` supposes the following logic:
        //   check to see if the user is in the router table AND
        //   (either does not have a router_type or the router type is the same as what they're currently using) AND
        //   (either there's no node_id assigned or the `connected_at` is earlier than the current connected_at)
        // Bigtable can't really do a few of those (e.g. detect non-existing values) so we have to simplify
        // things down a bit. We presume the record exists and that the `router_type` was specified when the
        // record was created.
        //
        // Filters are not very sophisticated on BigTable.
        // You can create RowFilterChains, where each filter acts as an "AND" or
        // a RowFilterUnion which acts as an "OR".
        //
        // There do not appear to be negative checks (e.g. check if not set)
        // According to [the docs](https://cloud.google.com/bigtable/docs/using-filters#chain)
        // ConditionalRowFilter is not atomic and changes can occur between predicate
        // and execution.
        //
        // We then use a ChainFilter (essentially an AND operation) to make sure that the router_type
        // matches and the new `connected_at` time is later than the existing `connected_at`

        let row = self.user_to_row(user);

        // === Router Filter Chain.
        let mut router_filter_chain = RowFilter_Chain::default();
        let mut filter_set: RepeatedField<RowFilter> = RepeatedField::default();
        // First check to see if the router type is either empty or matches exactly.
        // Yes, these are multiple filters. Each filter is basically an AND
        let mut filter = RowFilter::default();
        filter.set_family_name_regex_filter(ROUTER_FAMILY.to_owned());
        filter_set.push(filter);

        let mut filter = RowFilter::default();
        filter.set_column_qualifier_regex_filter("router_type".to_owned().as_bytes().to_vec());
        filter_set.push(filter);

        let mut filter = RowFilter::default();
        filter.set_value_regex_filter(user.router_type.as_bytes().to_vec());
        filter_set.push(filter);

        router_filter_chain.set_filters(filter_set);
        let mut router_filter = RowFilter::default();
        router_filter.set_chain(router_filter_chain);

        // === Connected_At filter chain
        let mut connected_filter_chain = RowFilter_Chain::default();
        let mut filter_set: RepeatedField<RowFilter> = RepeatedField::default();

        // then check to make sure that the last connected_at time is before this one.
        // Note: `check_and_mutate_row` uses `set_true_mutations`, meaning that only rows
        // that match the provided filters will be modified.
        let mut filter = RowFilter::default();
        filter.set_family_name_regex_filter(ROUTER_FAMILY.to_owned());
        filter_set.push(filter);

        let mut filter = RowFilter::default();
        filter.set_column_qualifier_regex_filter("connected_at".to_owned().as_bytes().to_vec());
        filter_set.push(filter);

        let mut filter = RowFilter::default();
        let mut val_range = ValueRange::default();
        val_range.set_start_value_open(0_u64.to_be_bytes().to_vec());
        val_range.set_end_value_open(user.connected_at.to_be_bytes().to_vec());
        filter.set_value_range_filter(val_range);
        filter_set.push(filter);

        connected_filter_chain.set_filters(filter_set);
        let mut connected_filter = RowFilter::default();
        connected_filter.set_chain(connected_filter_chain);

        // Gather the collections and try to update the row.

        let mut cond = RowFilter_Condition::default();
        cond.set_predicate_filter(router_filter);
        cond.set_true_filter(connected_filter);
        let mut cond_filter = RowFilter::default();
        cond_filter.set_condition(cond);
        // dbg!(&cond_filter);

        Ok(self.check_and_mutate_row(row, cond_filter).await?)
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let key = uaid.as_simple().to_string();
        let mut result = User {
            uaid: *uaid,
            ..Default::default()
        };

        if let Some(mut record) = self.read_row(&key, None).await? {
            trace!("🉑 Found a record for that user");
            if let Some(mut cells) = record.take_cells("connected_at") {
                if let Some(cell) = cells.pop() {
                    result.connected_at = to_u64(cell.value, "connected_at")?;
                }
            }

            if let Some(mut cells) = record.take_cells("router_type") {
                if let Some(cell) = cells.pop() {
                    result.router_type = String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?;
                }
            }

            if let Some(mut cells) = record.take_cells("router_data") {
                if let Some(cell) = cells.pop() {
                    result.router_data = from_str(&String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?)
                    .map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?;
                }
            }

            if let Some(mut cells) = record.take_cells("node_id") {
                if let Some(cell) = cells.pop() {
                    result.node_id = Some(String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?);
                }
            }

            if let Some(mut cells) = record.take_cells("record_version") {
                if let Some(cell) = cells.pop() {
                    result.record_version = Some(to_u64(cell.value, "record_version")?)
                }
            }

            if let Some(mut cells) = record.take_cells("current_timestamp") {
                if let Some(cell) = cells.pop() {
                    result.current_timestamp = Some(to_u64(cell.value, "current_timestamp")?)
                }
            }

            return Ok(Some(result));
        }
        Ok(None)
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let row_key = uaid.simple().to_string();
        self.delete_rows(&row_key).await?;
        Ok(())
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let key = format!("{}@{}", uaid.simple(), channel_id.as_hyphenated());

        // We can use the default timestamp here because there shouldn't be a time
        // based GC for router records.
        let mut row = Row {
            row_key: key,
            ..Default::default()
        };
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| DbError::General(e.to_string()))?
            .as_millis();
        row.cells.insert(
            ROUTER_FAMILY.to_owned(),
            vec![cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "updated".to_owned(),
                value: now.to_be_bytes().to_vec(),
                ..Default::default()
            }],
        );
        self.write_row(row).await.map_err(|e| e.into())
    }

    /// Add channels in bulk (used mostly during migration)
    ///
    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| DbError::General(e.to_string()))?
            .as_millis();
        let mut entries = protobuf::RepeatedField::default();
        let mut req = bigtable::MutateRowsRequest::default();
        let mut limit: u32 = 0;
        req.set_table_name(self.settings.table_name.clone());

        // Create entries that define rows that contain mutations to hold the updated value which
        // will create/update the channels.
        for channel in channels {
            let mut entry = bigtable::MutateRowsRequest_Entry::default();
            let key = format!("{}@{}", uaid.simple(), channel.as_hyphenated());
            entry.set_row_key(key.into_bytes());

            let mut cell_mutations = protobuf::RepeatedField::default();
            let mut mutation = data::Mutation::default();
            let mut set_cell = data::Mutation_SetCell {
                family_name: ROUTER_FAMILY.to_owned(),
                ..Default::default()
            };
            set_cell.set_column_qualifier("updated".as_bytes().to_vec());
            set_cell.set_value(now.to_be_bytes().to_vec());
            set_cell.set_timestamp_micros((now * 1000) as i64);

            mutation.set_set_cell(set_cell);
            cell_mutations.push(mutation);
            entry.set_mutations(cell_mutations);
            entries.push(entry);
            // There is a limit of 100,000 mutations per batch for bigtable.
            // https://cloud.google.com/bigtable/quotas
            // If you have 100,000 channels, you have too many.
            limit += 1;
            if limit >= 100_000 {
                break;
            }
        }
        req.set_entries(entries);

        let bigtable = self.pool.get().await?;

        // ClientSStreamReceiver will cancel an operation if it's dropped before it's done.
        let resp = bigtable
            .conn
            .mutate_rows(&req)
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
                                    )
                                    .into());
                                }
                                debug!("🎏 Response: {} OK", e.index);
                            }
                        }
                    }
                    Err(e) => return Err(error::BigTableError::Write(e).into()),
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

    /// Delete all the rows that start with the given prefix. NOTE: this may be metered and should
    /// be used with caution.
    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let mut result = HashSet::new();

        let mut req = bigtable::ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());

        let start_key = format!("{}@", uaid.simple());
        // '[' is the char after '@'
        let end_key = format!("{}[", uaid.simple());
        let mut rows = data::RowSet::default();
        let mut row_range = data::RowRange::default();
        row_range.set_start_key_open(start_key.into_bytes());
        row_range.set_end_key_open(end_key.into_bytes());
        let mut row_ranges = RepeatedField::default();
        row_ranges.push(row_range);
        rows.set_row_ranges(row_ranges);
        req.set_rows(rows);

        let mut strip_filter = data::RowFilter::default();
        strip_filter.set_strip_value_transformer(true);
        req.set_filter(strip_filter);

        let rows = self.read_rows(req, None, None).await?;
        for key in rows.keys().map(|v| v.to_owned()).collect::<Vec<String>>() {
            if let Some(chid) = key.split('@').last() {
                result.insert(Uuid::from_str(chid).map_err(|e| DbError::General(e.to_string()))?);
            }
        }

        Ok(result)
    }

    /// Delete the channel and all associated pending messages.
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let row_key = format!("{}@{}", uaid.simple(), channel_id.as_hyphenated());
        Ok(self.delete_rows(&row_key).await?)
    }

    /// Remove the node_id. Can't really "surgically strike" this
    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        _node_id: &str,
        connected_at: u64,
    ) -> DbResult<bool> {
        trace!(
            "🉑 Removing node_ids for {} up to {:?} ",
            &uaid.simple().to_string(),
            UNIX_EPOCH + Duration::from_secs(connected_at)
        );
        let row_key = uaid.simple().to_string();
        let mut time_range = data::TimestampRange::default();
        // convert connected at seconds into microseconds
        time_range.set_end_timestamp_micros((connected_at * 1000000) as i64);
        self.delete_cells(
            &row_key,
            ROUTER_FAMILY,
            &["node_id"].to_vec(),
            Some(&time_range),
        )
        .await?;
        Ok(true)
    }

    /// Write the notification to storage.
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let row_key = format!("{}#{}", uaid.simple(), message.chidmessageid());
        debug!("🗄️ Saving message {} :: {:?}", &row_key, &message);
        trace!(
            "🉑 timestamp: {:?}",
            &message.timestamp.to_be_bytes().to_vec()
        );
        let mut row = Row {
            row_key,
            ..Default::default()
        };

        // Remember, `timestamp` is effectively the time to kill the message, not the
        // current time.
        let ttl = SystemTime::now() + Duration::from_secs(message.ttl);
        trace!(
            "🉑 Message Expiry {}",
            ttl.duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        );

        let mut cells: Vec<cell::Cell> = Vec::new();

        let family = if let Some(topic) = message.topic {
            // Set the correct flag so we know how to read this row later.
            cells.push(cell::Cell {
                family: MESSAGE_TOPIC_FAMILY.to_owned(),
                qualifier: "topic".to_owned(),
                value: topic.into_bytes(),
                timestamp: ttl,
                ..Default::default()
            });
            MESSAGE_TOPIC_FAMILY
        } else {
            MESSAGE_FAMILY
        };
        let expiry: u128 = ttl
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        cells.extend(vec![
            cell::Cell {
                family: family.to_owned(),
                qualifier: "ttl".to_owned(),
                value: message.ttl.to_be_bytes().to_vec(),
                timestamp: ttl,
                ..Default::default()
            },
            cell::Cell {
                family: family.to_owned(),
                qualifier: "channel_id".to_owned(),
                value: message.channel_id.as_hyphenated().to_string().into_bytes(),
                timestamp: ttl,
                ..Default::default()
            },
            cell::Cell {
                family: family.to_owned(),
                qualifier: "timestamp".to_owned(),
                value: message.timestamp.to_be_bytes().to_vec(),
                timestamp: ttl,
                ..Default::default()
            },
            cell::Cell {
                family: family.to_owned(),
                qualifier: "version".to_owned(),
                value: message.version.into_bytes(),
                timestamp: ttl,
                ..Default::default()
            },
            cell::Cell {
                family: family.to_owned(),
                qualifier: "expiry".to_owned(),
                value: expiry.to_be_bytes().to_vec(),
                timestamp: ttl,
                ..Default::default()
            },
        ]);
        if let Some(headers) = message.headers {
            if !headers.is_empty() {
                cells.push(cell::Cell {
                    family: family.to_owned(),
                    qualifier: "headers".to_owned(),
                    value: json!(headers).to_string().into_bytes(),
                    timestamp: ttl,
                    ..Default::default()
                });
            }
        }
        if let Some(data) = message.data {
            cells.push(cell::Cell {
                family: family.to_owned(),
                qualifier: "data".to_owned(),
                value: data.into_bytes(),
                timestamp: ttl,
                ..Default::default()
            });
        }
        if let Some(sortkey_timestamp) = message.sortkey_timestamp {
            cells.push(cell::Cell {
                family: family.to_owned(),
                qualifier: "sortkey_timestamp".to_owned(),
                value: sortkey_timestamp.to_be_bytes().to_vec(),
                timestamp: ttl,
                ..Default::default()
            });
        }
        row.add_cells(family, cells);
        trace!("🉑 Adding row");
        self.write_row(row).await.map_err(|e| e.into())
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
        let mut row = Row {
            row_key,
            ..Default::default()
        };

        row.cells.insert(
            MESSAGE_FAMILY.to_owned(),
            vec![cell::Cell {
                family: MESSAGE_FAMILY.to_owned(),
                qualifier: "current_timestamp".to_owned(),
                value: timestamp.to_be_bytes().to_vec(),
                ..Default::default()
            }],
        );
        self.write_row(row).await.map_err(|e| e.into())
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
        self.delete_row(&row_key).await.map_err(|e| e.into())
    }

    /// Return `limit` pending messages from storage. `limit=0` for all messages.
    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let mut req = ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());

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

        req.set_filter(timestamp_filter()?);
        // Note set_rows_limit(v) limits the returned results
        // If you're doing additional filtering later, this is not what
        // you want.
        /*
        if limit > 0 {
            trace!("🉑 Setting limit to {limit}");
            req.set_rows_limit(limit as i64);
        }
        // */
        let rows = self.read_rows(req, None, Some(limit)).await?;
        debug!(
            "🉑 Fetch Topic Messages. Found {} row(s) of {}",
            rows.len(),
            limit
        );
        self.rows_to_notifications(rows, if limit > 0 { Some(limit) } else { None })
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

        let mut rows = data::RowSet::default();
        let mut row_range = data::RowRange::default();

        let start_key = if let Some(ts) = timestamp {
            format!("{}#02:{}", uaid.simple(), ts)
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
        req.set_filter(timestamp_filter()?);
        // Note set_rows_limit(v) limits the returned results from Bigtable.
        // If you're doing additional filtering later, this may not be what
        // you want and may artificially truncate possible return sets.
        /*
        if limit > 0 {
            req.set_rows_limit(limit as i64);
        }
        // */
        let rows = self.read_rows(req, timestamp, Some(limit)).await?;
        debug!(
            "🉑 Fetch Timestamp Messages ({:?}) Found {} row(s) of {}",
            timestamp,
            rows.len(),
            limit,
        );
        self.rows_to_notifications(rows, if limit > 0 { Some(limit) } else { None })
    }

    async fn health_check(&self) -> DbResult<bool> {
        self.pool
            .get()
            .await?
            .health_check(&self.settings.table_name)
            .await
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
    use crate::db::DbSettings;

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

        let connected_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

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
        client.remove_channel(&uaid, &chid_to_remove).await?;
        new_channels.remove(&chid_to_remove);
        let channels = client.get_channels(&uaid).await?;
        assert_eq!(channels, new_channels);

        // now ensure that we can update a user that's after the time we set prior.
        // first ensure that we can't update a user that's before the time we set prior.
        let updated = User {
            connected_at: fetched.connected_at - 300,
            ..test_user.clone()
        };
        let result = client.update_user(&updated).await;
        assert!(result.is_ok());
        assert!(!result.unwrap());

        // Make sure that the `connected_at` wasn't modified
        let fetched2 = client.get_user(&fetched.uaid).await?.unwrap();
        assert_eq!(fetched.connected_at, fetched2.connected_at);

        // and make sure we can update a record with a later connected_at time.
        let updated = User {
            connected_at: fetched.connected_at + 300,
            ..test_user
        };
        let result = client.update_user(&updated).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_ne!(
            test_user.connected_at,
            client.get_user(&uaid).await?.unwrap().connected_at
        );

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

        assert!(client
            .remove_node_id(&uaid, &node_id, connected_at)
            .await
            .is_ok());
        // did we remove it?
        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await?
            .messages;
        assert!(msgs.is_empty());

        assert!(client.remove_user(&uaid).await.is_ok());

        assert!(client.get_user(&uaid).await?.is_none());

        Ok(())
    }

    #[actix_rt::test]
    async fn read_cells_family_id() {
        let uaid = Uuid::parse_str(TEST_USER).unwrap();
        let client = new_client().unwrap();
        let _ = client.remove_user(&uaid).await.unwrap();

        let row_key = uaid.simple().to_string();

        let mut row = Row {
            row_key: row_key.clone(),
            ..Default::default()
        };
        row.cells.insert(
            ROUTER_FAMILY.to_owned(),
            vec![cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "foo".to_owned(),
                value: "bar".as_bytes().to_vec(),
                ..Default::default()
            }],
        );
        client.write_row(row).await.unwrap();
        let Some(row) = client.read_row(&row_key, None).await.unwrap() else {
            panic!("Expected row");
        };
        assert_eq!(row.cells.len(), 1);
        assert_eq!(row.cells.keys().next().unwrap(), ROUTER_FAMILY);
    }

    // #[actix_rt::test]
    // async fn sometest() {}

    // #[test]
    // fn sometest () {}
}
