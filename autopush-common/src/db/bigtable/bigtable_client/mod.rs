use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use google_cloud_rust_raw::bigtable::v2::bigtable;
use google_cloud_rust_raw::bigtable::v2::data;
use google_cloud_rust_raw::bigtable::v2::{
    bigtable::ReadRowsRequest, bigtable_grpc::BigtableClient,
};
use grpcio::{ChannelBuilder, ChannelCredentials, Environment};
use protobuf::RepeatedField;

pub mod cell;
pub mod error;
pub(crate) mod merge;
pub mod row;

// these are normally Vec<u8>
pub type RowKey = String;
pub type Qualifier = String;
// This must be a String.
pub type FamilyId = String;

#[derive(Clone)]
/// Wrapper for the BigTable connection
pub struct BigTableClient {
    /// The name of the table. This is used when generating a request.
    pub(crate) table_name: String,
    /// The client connection to BigTable.
    client: BigtableClient,
}

impl BigTableClient {
    pub fn new(
        env: Arc<Environment>,
        creds: ChannelCredentials,
        endpoint: &str,
        table_name: &str,
    ) -> Self {
        let chan = ChannelBuilder::new(env)
            .max_send_message_len(1 << 28)
            .max_receive_message_len(1 << 28)
            .set_credentials(creds)
            .connect(endpoint);

        Self {
            table_name: table_name.to_owned(),
            client: BigtableClient::new(chan),
        }
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
        column_names: &Vec<Vec<u8>>,
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
            del_cell.set_column_qualifier(column.to_owned());
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
