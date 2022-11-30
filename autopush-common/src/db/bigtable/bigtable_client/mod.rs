use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cadence::StatsdClient;
use google_cloud_rust_raw::bigtable::v2::bigtable;
use google_cloud_rust_raw::bigtable::v2::data;
use google_cloud_rust_raw::bigtable::v2::{
    bigtable::ReadRowsRequest, bigtable_grpc::BigtableClient,
};
use grpcio::{ChannelBuilder, ChannelCredentials, EnvBuilder};
use protobuf::RepeatedField;
use serde_json::{from_str, json};
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, UserRecord,
};

use self::cell::Cell;
use self::row::Row;

use super::BigTableError;

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
const MESSAGE_FAMILY: &str = "message";
const MESSAGE_TOPIC_FAMILY: &str = "message_topic";

#[derive(Clone)]
/// Wrapper for the BigTable connection
pub struct BigTableClientImpl {
    /// The name of the table. This is used when generating a request.
    pub(crate) table_name: String,
    /// The client connection to BigTable.
    client: BigtableClient,
}

impl BigTableClientImpl {
    pub fn new(_metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        let channel_creds = ChannelCredentials::google_default_credentials()
            .map_err(|e| DbError::ConnectionError(e.to_string()))?;
        let env = Arc::new(EnvBuilder::new().build());
        let endpoint = match &settings.dsn {
            Some(v) => v,
            None => {
                return Err(DbError::ConnectionError(
                    "No DSN specified in settings".to_owned(),
                ))
            }
        };
        let table_name = &settings.message_tablename;
        let chan = ChannelBuilder::new(env)
            .max_send_message_len(1 << 28)
            .max_receive_message_len(1 << 28)
            .set_credentials(channel_creds)
            .connect(endpoint);

        Ok(Self {
            table_name: table_name.to_owned(),
            client: BigtableClient::new(chan),
        })
    }

    /// Read a given row from the row key.
    pub async fn read_row(&self, row_key: &str) -> Result<Option<row::Row>, error::BigTableError> {
        debug!("Row key: {}", row_key);

        let mut row_keys = RepeatedField::default();
        row_keys.push(row_key.to_owned().as_bytes().to_vec());

        let mut row_set = data::RowSet::default();
        row_set.set_row_keys(row_keys);

        let mut req = bigtable::ReadRowsRequest::default();
        req.set_table_name(self.table_name.clone());
        req.set_rows(row_set);

        let rows = self.read_rows(req).await?;
        Ok(rows.get(row_key).cloned())
    }

    /// Take a big table ReadRowsRequest (containing the keys and filters) and return a set of row data.
    ///
    ///
    pub async fn read_rows(
        &self,
        req: ReadRowsRequest,
    ) -> Result<HashMap<RowKey, row::Row>, error::BigTableError> {
        let resp = self
            .client
            .clone()
            .read_rows(&req)
            .map_err(|e| error::BigTableError::BigTableRead(e.to_string()))?;
        merge::RowMerger::process_chunks(resp).await
    }

    /// write a given row.
    ///
    /// there's also `.mutate_rows` which I presume allows multiple.
    pub async fn write_row(&self, row: row::Row) -> Result<(), error::BigTableError> {
        let mut req = bigtable::MutateRowRequest::default();

        // compile the mutations.
        // It's possible to do a lot here, including altering in process
        // mutations, clearing them, etc. It's all up for grabs until we commit
        // below. For now, let's just presume a write and be done.
        let mut mutations = protobuf::RepeatedField::default();
        req.set_table_name(self.table_name.clone());
        req.set_row_key(row.row_key.into_bytes());
        for (_family, cells) in row.cells {
            for cell in cells {
                let mut mutation = data::Mutation::default();
                let mut set_cell = data::Mutation_SetCell::default();
                let timestamp = cell
                    .timestamp
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?;
                set_cell.family_name = cell.family;
                set_cell.set_column_qualifier(cell.qualifier.into_bytes());
                set_cell.set_value(cell.value);
                // Yes, this is passing milli bounded time as a micro. Otherwise I get
                // a `Timestamp granularity mismatch` error
                set_cell.set_timestamp_micros((timestamp.as_millis() * 1000) as i64);
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
            .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?
            .await
            .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?;
        Ok(())
    }

    /// Delete all cell data from the specified columns with the optional time range.
    pub async fn delete_cells(
        &self,
        row_key: &str,
        family: &str,
        column_names: &Vec<&str>,
        time_range: Option<&data::TimestampRange>,
    ) -> Result<(), error::BigTableError> {
        let mut req = bigtable::MutateRowRequest::default();
        req.set_table_name(self.table_name.clone());
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
            .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?
            .await
            .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?;
        Ok(())
    }

    /// Delete all the cells for the given row. NOTE: This will drop the row.
    pub async fn delete_row(&self, row_key: &str) -> Result<(), error::BigTableError> {
        let mut req = bigtable::MutateRowRequest::default();
        req.set_table_name(self.table_name.clone());
        let mut mutations = protobuf::RepeatedField::default();
        req.set_row_key(row_key.to_owned().into_bytes());
        let mut mutation = data::Mutation::default();
        mutation.set_delete_from_row(data::Mutation_DeleteFromRow::default());
        mutations.push(mutation);
        req.set_mutations(mutations);

        let _resp = self
            .client
            .mutate_row_async(&req)
            .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?
            .await
            .map_err(|e| error::BigTableError::BigTableWrite(e.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl DbClient for BigTableClientImpl {
    /// add user to the database
    async fn add_user(&self, user: &UserRecord) -> DbResult<()> {
        let mut row = Row {
            row_key: user.uaid.simple().to_string(),
            ..Default::default()
        };

        // TODO: These should probably be macros.
        row.add_cell(
            ROUTER_FAMILY,
            "connected_at",
            &user.connected_at.to_be_bytes().to_vec(),
            None,
        )
        .map_err(|e| DbError::Serialization(format!("Could not write connected_at {:?}", e)))?;
        row.add_cell(
            ROUTER_FAMILY,
            "router_type",
            &user.router_type.into_bytes().to_vec(),
            None,
        )
        .map_err(|e| DbError::Serialization(format!("Could not write router_type {:?}", e)))?;
        if let Some(router_data) = user.router_data {
            row.add_cell(
                ROUTER_FAMILY,
                "router_data",
                &json!(user.router_data).to_string().as_bytes().to_vec(),
                None,
            )
            .map_err(|e| DbError::Serialization(format!("Could not write router_data {:?}", e)))?;
        };
        if let Some(last_connect) = user.last_connect {
            row.add_cell(
                ROUTER_FAMILY,
                "last_connect",
                &last_connect.to_be_bytes().to_vec(),
                None,
            )
            .map_err(|e| DbError::Serialization(format!("Could not write last_connect {:?}", e)))?;
        };
        if let Some(node_id) = user.node_id {
            row.add_cell(
                ROUTER_FAMILY,
                "node_id",
                &node_id.into_bytes().to_vec(),
                None,
            )
            .map_err(|e| DbError::Serialization(format!("Could not write node_id {:?}", e)))?;
        };
        if let Some(record_version) = user.record_version {
            row.add_cell(
                ROUTER_FAMILY,
                "record_version",
                &record_version.to_be_bytes().to_vec(),
                None,
            )
            .map_err(|e| {
                DbError::Serialization(format!("Could not write record_version {:?}", e))
            })?;
        };
        if let Some(current_month) = user.current_month {
            row.add_cell(
                ROUTER_FAMILY,
                "current_month",
                &current_month.into_bytes().to_vec(),
                None,
            )
            .map_err(|e| {
                DbError::Serialization(format!("Could not write current_month {:?}", e))
            })?;
        };
        trace!("Adding user");
        self.write_row(row).await.map_err(|e| e.into())
    }

    /// BigTable doesn't really have the concept of an "update". You simply write the data and
    /// the individual cells create a new version. Depending on the garbage collection rules for
    /// the family, these can either persist or be automatically deleted.
    async fn update_user(&self, user: &UserRecord) -> DbResult<()> {
        self.add_user(user).await
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<UserRecord>> {
        let key = uaid.as_simple().to_string();
        let mut result = UserRecord {
            uaid: uaid.clone(),
            ..Default::default()
        };

        if let Some(record) = self.read_row(&key).await? {
            trace!("Found a record for that user");
            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "connected_at") {
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

            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "router_type") {
                if let Some(cell) = cells.pop() {
                    result.router_type = String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?;
                }
            }

            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "router_data") {
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

            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "last_connect") {
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

            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "node_id") {
                if let Some(cell) = cells.pop() {
                    result.node_id = Some(String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize router_type: {:?}",
                            e
                        ))
                    })?);
                }
            }

            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "record_version") {
                if let Some(cell) = cells.pop() {
                    if let Some(b) = cell.value.pop() {
                        result.record_version = Some(b)
                    }
                }
            }

            if let Some(cells) = record.get_cells(ROUTER_FAMILY, "current_month") {
                if let Some(cell) = cells.pop() {
                    result.node_id = Some(String::from_utf8(cell.value).map_err(|e| {
                        DbError::Serialization(format!(
                            "Could not deserialize current_month: {:?}",
                            e
                        ))
                    })?);
                }
            }

            return Ok(Some(result));
        }
        Ok(None)
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let channels = self.get_channels(&uaid).await?;
        for channel in channels {
            self.delete_row(&channel.simple().to_string()).await?;
        }
        self.delete_row(&uaid.simple().to_string())
            .await
            .map_err(|e| e.into())
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let key = format!(
            "{}#{}",
            uaid.simple().to_string(),
            channel_id.simple().to_string()
        );

        let mut row = Row {
            row_key: key,
            ..Default::default()
        };

        // rows disappear from bigtable if they're empty, so give it at least one "real" value.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| DbError::General(e.to_string()))?;
        row.add_cell(
            ROUTER_FAMILY,
            "updated",
            &now.as_millis().to_be_bytes().to_vec(),
            None,
        )?;

        self.write_row(row).await.map_err(|e| e.into())
    }

    async fn save_channels(
        &self,
        uaid: &Uuid,
        channel_list: HashSet<&Uuid>,
        _message_month: &str,
    ) -> DbResult<()> {
        if channel_list.is_empty() {
            trace!("No channels to save.");
            return Ok(());
        };

        for channel in channel_list {
            trace!(
                "+ Saving channel {:?}: {:?}",
                &uaid.simple().to_string(),
                &channel.simple().to_string()
            );
            self.add_channel(uaid, channel).await?;
        }

        Ok(())
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
                regex_filter.set_row_key_regex_filter(format!("^{}#", uaid).as_bytes().to_vec());

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
            req.set_table_name(self.table_name.clone());
            req.set_filter(filter);

            req
        };

        for key in self
            .read_rows(req)
            .await?
            .keys()
            .map(|v| v.to_owned())
            .collect::<Vec<String>>()
        {
            result.insert(Uuid::from_str(&key).map_err(|e| DbError::General(e.to_string()))?);
        }

        Ok(result)
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let row_key = format!(
            "{}#{}",
            uaid.simple().to_string(),
            channel_id.simple().to_string()
        );
        self.delete_row(&row_key).await?;
        Ok(true)
    }

    /// Remove the node_id. Can't really "surgically strike" this
    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        trace!(
            "Removing node_ids for {} up to {:?} ",
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
        .await
        .map_err(|e| e.into())
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let mut row = Row {
            row_key: format!(
                "{}#{}",
                uaid.simple().to_string(),
                message.channel_id.simple().to_string()
            ),
            ..Default::default()
        };

        let ttl = SystemTime::now() + Duration::from_secs(message.ttl);

        let family = if message.topic.is_some() {
            MESSAGE_TOPIC_FAMILY
        } else {
            MESSAGE_FAMILY
        };

        row.add_cell(
            family,
            "version",
            &message.version.into_bytes().to_vec(),
            Some(ttl),
        )?;
        if let Some(topic) = message.topic {
            row.add_cell(family, "topic", &topic.into_bytes().to_vec(), Some(ttl))?;
        };
        row.add_cell(
            family,
            "timestamp",
            &message.timestamp.to_be_bytes().to_vec(),
            Some(ttl),
        );
        if let Some(data) = message.data {
            row.add_cell(family, "data", &data.into_bytes().to_vec(), Some(ttl));
        }
        if let Some(sortkey_timestamp) = message.sortkey_timestamp {
            row.add_cell(
                family,
                "sortkey_timestamp",
                &sortkey_timestamp.to_be_bytes().to_vec(),
                Some(ttl),
            );
        }
        row.add_cell(
            family,
            "headers",
            &json!(message.headers).to_string().into_bytes().to_vec(),
            Some(ttl),
        );

        trace!("Adding row");
        self.write_row(row).await.map_err(|e| e.into())
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        // parse the sort_key to get the message's CHID
        let parts: Vec<&str> = sort_key.split(':').collect();
        let family = match parts.pop() {
            Some("01") => MESSAGE_TOPIC_FAMILY,
            Some("02") => MESSAGE_FAMILY,
            _ => {
                return Err(DbError::General(format!(
                    "Invalid sort_key detected: {}",
                    sort_key
                )))
            }
        };
        let channel_id = match parts.pop() {
            Some(v) => v,
            None => {
                return Err(DbError::General(format!(
                    "Invalid sort_key detected: {}",
                    sort_key
                )))
            }
        };
        let row_key = format!("{}#{}", uaid.simple().to_string(), channel_id);
        self.delete_cells(&row_key, family, &["data"].to_vec(), None)
            .await
            .map_err(|e| e.into())
    }

    async fn fetch_messages(&self, uaid: &Uuid, limit: usize) -> DbResult<FetchMessageResponse> {
        // TODO

    }

    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        // TODO

    }
}
