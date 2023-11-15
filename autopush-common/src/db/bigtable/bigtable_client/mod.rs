use core::fmt;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Display;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cadence::StatsdClient;
use google_cloud_rust_raw::bigtable::admin::v2::bigtable_table_admin::DropRowRangeRequest;
use google_cloud_rust_raw::bigtable::admin::v2::bigtable_table_admin_grpc::BigtableTableAdminClient;
use google_cloud_rust_raw::bigtable::v2::{bigtable, data};
use google_cloud_rust_raw::bigtable::v2::{
    bigtable::ReadRowsRequest, bigtable_grpc::BigtableClient,
};
use grpcio::{Channel, ChannelBuilder, ChannelCredentials, EnvBuilder};
use protobuf::RepeatedField;
use serde_json::{from_str, json};
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, User,
};
use crate::notification::STANDARD_NOTIFICATION_PREFIX;

use self::row::Row;
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
    /// The grpc client connection to BigTable (it carries no big table specific info)
    client: BigtableClient,
    /// Metrics client
    _metrics: Arc<StatsdClient>,
    /// Connection Channel
    chan: Channel,
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

/// Create a normalized index key.
fn as_key(uaid: &Uuid, channel_id: Option<&Uuid>, chidmessageid: Option<&str>) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(uaid.simple().to_string());
    if let Some(channel_id) = channel_id {
        parts.push(channel_id.simple().to_string());
    } else if chidmessageid.is_some() {
        parts.push("".to_string())
    }
    if let Some(chidmessageid) = chidmessageid {
        parts.push(chidmessageid.to_owned());
    }
    parts.join("#")
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
        let env = Arc::new(EnvBuilder::new().build());
        let endpoint = match &settings.dsn {
            Some(v) => v,
            None => {
                return Err(DbError::ConnectionError(
                    "No DSN specified in settings".to_owned(),
                ))
            }
        };
        debug!("🉑 DSN: {}", &endpoint);
        let parsed = url::Url::parse(endpoint).map_err(|e| {
            DbError::ConnectionError(format!("Invalid DSN: {:?} : {:?}", endpoint, e))
        })?;
        // Url::parsed() doesn't know how to handle `grpc:` schema, so it returns "null".
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
        let db_settings = BigTableDbSettings::try_from(settings.db_settings.as_ref())?;
        debug!("🉑 {:#?}", db_settings);
        let mut chan = ChannelBuilder::new(env)
            .max_send_message_len(1 << 28)
            .max_receive_message_len(1 << 28);
        // Don't get the credentials if we are running in the emulator
        if settings
            .dsn
            .clone()
            .map(|v| v.contains("localhost"))
            .unwrap_or(false)
            || std::env::var("BIGTABLE_EMULATOR_HOST").is_ok()
        {
            debug!("🉑 Using emulator");
        } else {
            chan = chan.set_credentials(
                ChannelCredentials::google_default_credentials()
                    .map_err(|e| DbError::ConnectionError(e.to_string()))?,
            );
            debug!("🉑 Using real");
        }

        let con_str = format!("{}{}", origin, parsed.path());
        debug!("🉑 connection string {}", &con_str);
        let chan = chan.connect(&con_str);
        let client = BigtableClient::new(chan.clone());
        Ok(Self {
            settings: db_settings,
            client,
            chan,
            _metrics: metrics,
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
        row_keys.push(row_key.to_owned().as_bytes().to_vec());

        let mut row_set = data::RowSet::default();
        row_set.set_row_keys(row_keys);

        let mut req = bigtable::ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_rows(row_set);

        let rows = self.read_rows(req, timestamp_filter, None).await?;
        Ok(rows.get(row_key).cloned())
    }

    /// Take a big table ReadRowsRequest (containing the keys and filters) and return a set of row data indexed by row key.
    ///
    ///
    async fn read_rows(
        &self,
        req: ReadRowsRequest,
        timestamp_filter: Option<u64>,
        limit: Option<usize>,
    ) -> Result<BTreeMap<RowKey, row::Row>, error::BigTableError> {
        let resp = self
            .client
            .read_rows(&req)
            .map_err(|e| error::BigTableError::Read(e.to_string()))?;
        merge::RowMerger::process_chunks(resp, timestamp_filter, limit).await
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
        let mut mutations = protobuf::RepeatedField::default();
        req.set_table_name(self.settings.table_name.clone());
        req.set_row_key(row.row_key.into_bytes());
        for (_family, cells) in row.cells {
            for cell in cells {
                let mut mutation = data::Mutation::default();
                let mut set_cell = data::Mutation_SetCell::default();
                let timestamp = cell
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(|e| error::BigTableError::Write(e.to_string()))?;
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
        req.set_mutations(mutations);

        // Do the actual commit.
        // fails with `cannot execute `LocalPool` executor from within another executor: EnterError`
        let _resp = self
            .client
            .mutate_row_async(&req)
            .map_err(|e| error::BigTableError::Write(e.to_string()))?
            .await
            .map_err(|e| error::BigTableError::Write(e.to_string()))?;
        Ok(())
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

        let _resp = self
            .client
            .mutate_row_async(&req)
            .map_err(|e| error::BigTableError::Write(e.to_string()))?
            .await
            .map_err(|e| error::BigTableError::Write(e.to_string()))?;
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

        let _resp = self
            .client
            .mutate_row_async(&req)
            .map_err(|e| error::BigTableError::Write(e.to_string()))?
            .await
            .map_err(|e| error::BigTableError::Write(e.to_string()))?;
        Ok(())
    }

    /// This uses the admin interface to drop row ranges.
    /// This will drop ALL data associated with these rows.
    /// Note that deletion may take up to a week to occur.
    /// see https://cloud.google.com/php/docs/reference/cloud-bigtable/latest/Admin.V2.DropRowRangeRequest
    async fn delete_rows(&self, row_key: &str) -> Result<bool, error::BigTableError> {
        let admin = BigtableTableAdminClient::new(self.chan.clone());
        let mut req = DropRowRangeRequest::new();
        req.set_name(self.settings.table_name.clone());
        req.set_row_key_prefix(row_key.as_bytes().to_vec());
        req.set_delete_all_data_from_table(true);
        admin
            .drop_row_range_async(&req)
            .map_err(|e| {
                error!("{:?}", e);
                error::BigTableError::Admin("Could not delete data from table (0)".to_owned())
            })?
            .await
            .map_err(|e| {
                error!("post await: {:?}", e);
                error::BigTableError::Admin("Could not delete data from table (1)".to_owned())
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
            if let Some(cell) = row.get_cell("channel_id") {
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
                if let Some(cell) = row.get_cell("version") {
                    notif.version = to_string(cell.value, "version")?;
                }
                if let Some(cell) = row.get_cell("topic") {
                    notif.topic = Some(to_string(cell.value, "topic")?);
                }

                if let Some(cell) = row.get_cell("ttl") {
                    notif.ttl = to_u64(cell.value, "ttl")?;
                }

                if let Some(cell) = row.get_cell("data") {
                    notif.data = Some(to_string(cell.value, "data")?);
                }
                if let Some(cell) = row.get_cell("sortkey_timestamp") {
                    let sk_ts = to_u64(cell.value, "sortkey_timestamp")?;
                    notif.sortkey_timestamp = Some(sk_ts);
                    if sk_ts > max_timestamp {
                        max_timestamp = sk_ts;
                    }
                }
                if let Some(cell) = row.get_cell("timestamp") {
                    notif.timestamp = to_u64(cell.value, "timestamp")?;
                }
                if let Some(cell) = row.get_cell("headers") {
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
}

#[async_trait]
impl DbClient for BigTableClientImpl {
    /// add user to the database
    async fn add_user(&self, user: &User) -> DbResult<()> {
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
        if let Some(last_connect) = user.last_connect {
            cells.push(cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "last_connect".to_owned(),
                value: last_connect.to_be_bytes().to_vec(),
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
                value: node_id.clone().into_bytes().to_vec(),
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
        if let Some(current_month) = &user.current_month {
            cells.push(cell::Cell {
                family: ROUTER_FAMILY.to_owned(),
                qualifier: "current_month".to_owned(),
                value: current_month.clone().into_bytes().to_vec(),
                ..Default::default()
            });
        };
        row.add_cells(ROUTER_FAMILY, cells);
        trace!("🉑 Adding user");
        self.write_row(row).await.map_err(|e| e.into())
    }

    /// BigTable doesn't really have the concept of an "update". You simply write the data and
    /// the individual cells create a new version. Depending on the garbage collection rules for
    /// the family, these can either persist or be automatically deleted.
    async fn update_user(&self, user: &User) -> DbResult<bool> {
        self.add_user(user).await?;
        // TODO: is a conditional check possible?
        Ok(true)
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let key = uaid.as_simple().to_string();
        let mut result = User {
            uaid: *uaid,
            ..Default::default()
        };

        if let Some(record) = self.read_row(&key, None).await? {
            trace!("🉑 Found a record for that user");
            if let Some(mut cells) = record.get_cells("connected_at") {
                if let Some(cell) = cells.pop() {
                    let v: [u8; 8] = cell.value.try_into().map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize connected_at: {:?}",
                            e
                        ))
                    })?;
                    result.connected_at = u64::from_be_bytes(v);
                }
            }

            if let Some(mut cells) = record.get_cells("router_type") {
                if let Some(cell) = cells.pop() {
                    result.router_type = String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?;
                }
            }

            if let Some(mut cells) = record.get_cells("router_data") {
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

            if let Some(mut cells) = record.get_cells("last_connect") {
                if let Some(cell) = cells.pop() {
                    let v: [u8; 8] = cell.value.try_into().map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize last_connect: {:?}",
                            e
                        ))
                    })?;
                    result.last_connect = Some(u64::from_be_bytes(v));
                }
            }

            if let Some(mut cells) = record.get_cells("node_id") {
                if let Some(cell) = cells.pop() {
                    result.node_id = Some(String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?);
                }
            }

            if let Some(mut cells) = record.get_cells("record_version") {
                if let Some(mut cell) = cells.pop() {
                    // there's only one byte, so pop it off and use it.
                    if let Some(b) = cell.value.pop() {
                        result.record_version = Some(b)
                    }
                }
            }

            if let Some(mut cells) = record.get_cells("current_month") {
                if let Some(cell) = cells.pop() {
                    result.current_month = Some(String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize current_month: {:?}",
                            e
                        ))
                    })?);
                }
            }

            //TODO: rename this to `last_notification_timestamp`
            if let Some(mut cells) = record.get_cells("current_timestamp") {
                if let Some(cell) = cells.pop() {
                    let v: [u8; 8] = cell.value.try_into().map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize current_timestamp: {:?}",
                            e
                        ))
                    })?;
                    result.current_timestamp = Some(u64::from_be_bytes(v));
                }
            }

            return Ok(Some(result));
        }
        Ok(None)
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        self.delete_rows(&as_key(uaid, None, None)).await?;
        Ok(())
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let key = as_key(uaid, Some(channel_id), None);

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

    /// Delete all the rows that start with the given prefix. NOTE: this may be metered and should
    /// be used with caution.
    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let mut result = HashSet::new();

        let req = {
            let filter = {
                let mut strip_filter = data::RowFilter::default();
                strip_filter.set_strip_value_transformer(true);
                let mut regex_filter = data::RowFilter::default();
                // Your regex expression must match the WHOLE string. No partial matches.
                // For this, we only want to grab the channel meta records (which do not
                // have a sort key suffix)
                let key = format!("^{}#[^#]+", uaid.simple());
                regex_filter.set_row_key_regex_filter(key.as_bytes().to_vec());
                let mut chain = data::RowFilter_Chain::default();
                let mut repeat_field = RepeatedField::default();
                repeat_field.push(strip_filter);
                repeat_field.push(regex_filter);
                chain.set_filters(repeat_field);

                let mut filter = data::RowFilter::default();
                filter.set_chain(chain);
                filter
            };

            let mut req = bigtable::ReadRowsRequest::default();
            req.set_table_name(self.settings.table_name.clone());
            req.set_filter(filter);

            req
        };

        let rows = self.read_rows(req, None, None).await?;
        for key in rows.keys().map(|v| v.to_owned()).collect::<Vec<String>>() {
            if let Some(chid) = key.split('#').last() {
                result.insert(Uuid::from_str(chid).map_err(|e| DbError::General(e.to_string()))?);
            }
        }

        Ok(result)
    }

    /// Delete the channel and all associated pending messages.
    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let row_key = as_key(uaid, Some(channel_id), None);
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
        let row_key = as_key(uaid, None, None);
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
        // TODO: is a conditional check possible?
        Ok(true)
    }

    /// Write the notification to storage.
    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let row_key = as_key(
            uaid,
            Some(&message.channel_id),
            Some(&message.chidmessageid()),
        );
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

        let family = if message.topic.is_some() {
            // Set the correct flag so we know how to read this row later.
            cells.push(cell::Cell {
                family: MESSAGE_FAMILY.to_owned(),
                qualifier: "has_topic".to_owned(),
                value: vec![1],
                timestamp: ttl,
                ..Default::default()
            });
            cells.push(cell::Cell {
                family: MESSAGE_TOPIC_FAMILY.to_owned(),
                qualifier: "topic".to_owned(),
                value: message.topic.unwrap().into_bytes().to_vec(),
                timestamp: ttl,
                ..Default::default()
            });
            MESSAGE_TOPIC_FAMILY
        } else {
            MESSAGE_FAMILY
        };
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
                value: ttl
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .to_be_bytes()
                    .to_vec(),
                timestamp: ttl,
                ..Default::default()
            },
        ]);
        if let Some(headers) = message.headers {
            if !headers.is_empty() {
                cells.push(cell::Cell {
                    family: family.to_owned(),
                    qualifier: "headers".to_owned(),
                    value: json!(headers).to_string().into_bytes().to_vec(),
                    timestamp: ttl,
                    ..Default::default()
                });
            }
        }
        if let Some(data) = message.data {
            cells.push(cell::Cell {
                family: family.to_owned(),
                qualifier: "data".to_owned(),
                value: data.into_bytes().to_vec(),
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
        let mut row = Row {
            row_key: as_key(uaid, None, None),
            ..Default::default()
        };

        debug!(
            "🉑 Updating {} current_timestamp:  {:?}",
            as_key(uaid, None, None),
            timestamp.to_be_bytes().to_vec()
        );

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
        // parse the sort_key to get the message's CHID
        let parts: Vec<&str> = chidmessageid.split(':').collect();
        if parts.len() < 3 {
            return Err(DbError::General(format!(
                "Invalid sort_key detected: {}",
                chidmessageid
            )));
        }
        let family = match parts[0] {
            "01" => MESSAGE_TOPIC_FAMILY,
            "02" => MESSAGE_FAMILY,
            _ => "",
        };
        if family.is_empty() {
            return Err(DbError::General(format!(
                "Invalid sort_key detected: {}",
                chidmessageid
            )));
        }
        let chid = Uuid::parse_str(parts[1])
            .map_err(|_| error::BigTableError::Admin("Invalid SortKey component".to_string()))?;
        let row_key = as_key(uaid, Some(&chid), Some(chidmessageid));
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
        req.set_filter({
            let mut regex_filter = data::RowFilter::default();
            // channels for a given UAID all begin with `{uaid}#`
            // this will fetch all messages for all channels and all sort_keys
            regex_filter.set_row_key_regex_filter(
                format!("^{}#[^#]+#01:.+", uaid.simple())
                    .as_bytes()
                    .to_vec(),
            );
            regex_filter
        });
        // Note set_rows_limit(v) limits the returned results from Bigtable.
        // If you're doing additional filtering later, this may not be what
        // you want and may artificially truncate possible return sets.
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

        // We can fetch data and do [some remote filtering](https://cloud.google.com/bigtable/docs/filters),
        // unfortunately I don't think the filtering we need will be super helpful.
        //
        //
        let filter = {
            // Only look for channelids for the given UAID.
            // start by looking for rows that roughly match what we want
            // Note: BigTable provides a good deal of specialized filtering, but
            // it tends to be overly specialized. (For instance, a value range retuns
            // cells which has values within a specific range. Not rows, not families,
            // cells. There does not appear to be a way to chain this so that it only
            // looks for rows with ranged values within a given family or qualifier types
            // That must be done externally.)
            let mut filter = data::RowFilter::default();
            // look for anything belonging to this UAID that is also a Standard Notification
            let pattern = format!(
                "^{}#[^#]+#{}:.*",
                uaid.simple(),
                STANDARD_NOTIFICATION_PREFIX,
            );
            trace!("🉑 regex filter {:?}", pattern);
            filter.set_row_key_regex_filter(pattern.as_bytes().to_vec());
            filter
        };
        req.set_filter(filter);
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
        let mut req = bigtable::ReadRowsRequest::default();
        req.set_table_name(self.settings.table_name.clone());
        let mut row = data::Row::default();
        // Pick an non-existant key.
        row.set_key("NOT_FOUND".to_owned().as_bytes().to_vec());
        // Block any possible results.
        let mut filter = data::RowFilter::default();
        filter.set_block_all_filter(true);
        req.set_filter(filter);
        // we don't care about the return (it's going to be empty) but we DO care if it fails.
        let _ = self
            .client
            .read_rows(&req)
            .map_err(|e| DbError::General(format!("BigTable connectivity error: {:?}", e)))?;

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

    /// BigTable does not support message table rotation
    fn rotating_message_table(&self) -> Option<&str> {
        None
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
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

    use super::*;
    use cadence::StatsdClient;

    use crate::db::DbSettings;

    const TEST_USER: &str = "DEADBEEF-0000-0000-0000-0123456789AB";
    const TEST_CHID: &str = "DECAFBAD-0000-0000-0000-0123456789AB";

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn new_client() -> DbResult<BigTableClientImpl> {
        let settings = DbSettings {
            // this presumes the table was created with
            // `cbt -project test -instance test createtable autopush`
            dsn: Some("grpc://localhost:8086".to_owned()),
            db_settings: json!({"table_name":"projects/test/instances/test/tables/autopush"})
                .to_string(),
        };

        let metrics = Arc::new(StatsdClient::builder("", cadence::NopMetricSink).build());

        BigTableClientImpl::new(metrics, &settings)
    }

    #[test]
    fn row_key() {
        let uaid = Uuid::parse_str(TEST_USER).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let chidmessageid = "01:decafbad-0000-0000-0000-0123456789ab:Inbox";
        let k = as_key(&uaid, Some(&chid), Some(chidmessageid));
        assert_eq!(k, "deadbeef0000000000000123456789ab#decafbad0000000000000123456789ab#01:decafbad-0000-0000-0000-0123456789ab:Inbox");
    }

    /// run a gauntlet of testing. These are a bit linear because they need
    /// to run in sequence.
    #[actix_rt::test]
    async fn run_gauntlet() {
        let client = new_client().unwrap();

        let connected_at = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let uaid = Uuid::parse_str(TEST_USER).unwrap();
        let chid = Uuid::parse_str(TEST_CHID).unwrap();
        let node_id = "test_node".to_owned();

        let test_user = User {
            uaid,
            router_type: "webpush".to_owned(),
            connected_at,
            router_data: None,
            last_connect: Some(connected_at),
            node_id: Some(node_id.clone()),
            ..Default::default()
        };

        // can we add the user?
        let user = client.add_user(&test_user).await;
        assert!(user.is_ok());
        let fetched = client.get_user(&uaid).await.unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().router_type, "webpush".to_owned());

        // can we add channels?
        client.add_channel(&uaid, &chid).await.unwrap();
        let channels = client.get_channels(&uaid).await;
        assert!(channels.unwrap().contains(&chid));

        // can we modify the user record?
        let updated = User {
            connected_at: now() + 3,
            ..test_user
        };
        assert!(client.update_user(&updated).await.is_ok());
        assert_ne!(
            test_user.connected_at,
            client.get_user(&uaid).await.unwrap().unwrap().connected_at
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
        assert!(client
            .save_message(&uaid, test_notification.clone())
            .await
            .is_ok());

        let mut fetched = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await
            .unwrap();
        assert_ne!(fetched.messages.len(), 0);
        let fm = fetched.messages.pop().unwrap();
        assert_eq!(fm.channel_id, test_notification.channel_id);
        assert_eq!(fm.data, Some(test_data));

        // Grab all 1 of the messages that were submmited within the past 10 seconds.
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(timestamp - 10), 999)
            .await
            .unwrap();
        assert_ne!(fetched.messages.len(), 0);

        // Try grabbing a message for 10 seconds from now.
        let fetched = client
            .fetch_timestamp_messages(&uaid, Some(timestamp + 10), 999)
            .await
            .unwrap();
        assert_eq!(fetched.messages.len(), 0);

        // can we clean up our toys?
        assert!(client
            .remove_message(&uaid, &format!("02:{}:{}", chid.as_simple(), sort_key))
            .await
            .is_ok());

        assert!(client.remove_channel(&uaid, &chid).await.is_ok());
        assert!(client
            .remove_node_id(&uaid, &node_id, connected_at)
            .await
            .is_ok());
        // did we remove it?
        let msgs = client
            .fetch_timestamp_messages(&uaid, None, 999)
            .await
            .unwrap()
            .messages;
        print!("Messages: {:?}", &msgs);
        assert!(msgs.is_empty());

        assert!(client.remove_user(&uaid).await.is_ok());

        assert!(client.get_user(&uaid).await.unwrap().is_none());
    }

    // #[actix_rt::test]
    // async fn sometest() {}

    // #[test]
    // fn sometest () {}
}
