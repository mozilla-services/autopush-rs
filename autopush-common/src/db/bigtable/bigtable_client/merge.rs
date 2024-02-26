use std::collections::{BTreeMap, HashMap};
use std::mem;
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use google_cloud_rust_raw::bigtable::v2::bigtable::{ReadRowsResponse, ReadRowsResponse_CellChunk};
use grpcio::ClientSStreamReceiver;

use super::{cell::Cell, error::BigTableError, row::Row, FamilyId, Qualifier, RowKey};

/// List of the potential states when we are reading each value from the
/// returned stream and composing a "row"
#[derive(Debug, Default, Clone, Eq, PartialEq)]
enum ReadState {
    #[default]
    RowStart,
    CellStart,
    CellInProgress,
    CellComplete,
    RowComplete,
}

/// An in-progress Cell data struct.
#[derive(Debug, Clone)]
pub(crate) struct PartialCell {
    pub(crate) family: FamilyId,
    /// A "qualifier" is a column name
    pub(crate) qualifier: Qualifier,
    /// Timestamps are returned as microseconds, but need to be
    /// specified as milliseconds (even though the function asks
    /// for microseconds, you * 1000 the mls).
    pub(crate) timestamp: SystemTime,
    /// Not sure if or how these are used
    pub(crate) labels: Vec<String>,
    /// The data buffer.
    pub(crate) value: Vec<u8>,
    /// the returned sort order for the cell.
    pub(crate) value_index: usize,
}

impl Default for PartialCell {
    fn default() -> Self {
        Self {
            family: String::default(),
            qualifier: String::default(),
            timestamp: SystemTime::now(),
            labels: Vec::new(),
            value: Vec::new(),
            value_index: 0,
        }
    }
}

/// An in-progress Row structure
#[derive(Debug, Default, Clone)]
pub(crate) struct PartialRow {
    /// table uniquie key
    row_key: RowKey,
    /// map of cells per family identifier.
    cells: HashMap<FamilyId, Vec<PartialCell>>,
    /// the last family id string we encountered
    last_family: FamilyId,
    /// the working set of cells for a given qualifier
    /// *NOTE* Bigtable wants to collect cells by Family ID. That's
    /// not important for us to process data, so we cheat a bit and
    /// return a row structure that replaces the Family ID with the
    /// cell qualifier, allowing us to index the returned data faster.
    last_collected_cells: HashMap<Qualifier, Vec<Cell>>,
    /// the last column name we've encountered
    last_qualifier: Qualifier,
    /// Any cell that may be in progress (chunked
    /// across multiple portions)
    cell_in_progress: PartialCell,
}

/// workhorse struct, this is used to gather item data from the stream and build rows.
#[derive(Debug, Default)]
pub struct RowMerger {
    /// The row's current state. State progresses while processing a single chunk.
    state: ReadState,
    /// The last row key we've encountered. This should be consistent across the chunks.
    last_seen_row_key: Option<RowKey>,
    /// The last cell family. This may change, indicating a new cell group.
    last_seen_cell_family: Option<FamilyId>,
    /// The row that is currently being compiled.
    row_in_progress: PartialRow,
}

impl RowMerger {
    /// discard data so far and return to a neutral state.
    fn reset_row(&mut self, chunk: ReadRowsResponse_CellChunk) -> Result<&mut Self, BigTableError> {
        if self.state == ReadState::RowStart {
            return Err(BigTableError::InvalidChunk("Bare Reset".to_owned()));
        };
        if !chunk.row_key.is_empty() {
            return Err(BigTableError::InvalidChunk(
                "Reset chunk has a row key".to_owned(),
            ));
        };
        if chunk.has_family_name() {
            return Err(BigTableError::InvalidChunk(
                "Reset chunk has a family_name".to_owned(),
            ));
        }
        if chunk.has_qualifier() {
            return Err(BigTableError::InvalidChunk(
                "Reset chunk has a qualifier".to_owned(),
            ));
        }
        if chunk.timestamp_micros > 0 {
            return Err(BigTableError::InvalidChunk(
                "Reset chunk has a timestamp".to_owned(),
            ));
        }
        if !chunk.get_labels().is_empty() {
            return Err(BigTableError::InvalidChunk(
                "Reset chunk has a labels".to_owned(),
            ));
        }
        if !chunk.value.is_empty() {
            return Err(BigTableError::InvalidChunk(
                "Reset chunk has value".to_owned(),
            ));
        }

        self.state = ReadState::RowStart;
        self.row_in_progress = PartialRow::default();
        Ok(self)
    }

    /// The initial row contains the first cell data. There may be additional data that we
    /// have to use later, so capture that as well.
    fn row_start(
        &mut self,
        chunk: &mut ReadRowsResponse_CellChunk,
    ) -> Result<&Self, BigTableError> {
        if chunk.row_key.is_empty() {
            return Err(BigTableError::InvalidChunk(
                "New row is missing a row key".to_owned(),
            ));
        }
        if chunk.has_family_name() {
            info!(
                "👪Family name: {}: {:?}",
                String::from_utf8(chunk.row_key.clone()).unwrap_or_default(),
                &chunk.get_family_name()
            );
            self.last_seen_cell_family = Some(chunk.get_family_name().get_value().to_owned());
        }
        if let Some(last_key) = self.last_seen_row_key.clone() {
            if last_key.as_bytes().to_vec() >= chunk.row_key {
                return Err(BigTableError::InvalidChunk(
                    "Out of order row keys".to_owned(),
                ));
            }
        }

        let row = &mut self.row_in_progress;

        row.row_key = String::from_utf8(chunk.row_key.clone()).unwrap_or_default();
        row.cell_in_progress = PartialCell::default();

        self.state = ReadState::CellStart;
        Ok(self)
    }

    /// cell_start seems to be the main worker. It starts a new cell value (rows contain cells, which
    /// can have multiple versions).
    fn cell_start(
        &mut self,
        chunk: &mut ReadRowsResponse_CellChunk,
    ) -> Result<&Self, BigTableError> {
        // cells must have qualifiers.
        if !chunk.has_qualifier() {
            self.state = ReadState::CellComplete;
            return Ok(self);
            // return Err(BigTableError::InvalidChunk("Cell missing qualifier for new cell".to_owned()))
        }
        let qualifier = chunk.take_qualifier().get_value().to_vec();
        // dbg!(chunk.has_qualifier(), String::from_utf8(qualifier.clone()));
        let row = &mut self.row_in_progress;

        if !row.cells.is_empty()
            && !chunk.row_key.is_empty()
            && chunk.row_key != row.row_key.as_bytes()
        {
            return Err(BigTableError::InvalidChunk(
                "Row key changed mid row".to_owned(),
            ));
        }

        let cell = &mut row.cell_in_progress;
        if chunk.has_family_name() {
            cell.family = chunk.take_family_name().get_value().to_owned();
        } else {
            if self.last_seen_cell_family.is_none() {
                return Err(BigTableError::InvalidChunk(
                    "Cell missing family for new cell".to_owned(),
                ));
            }
            cell.family = self.last_seen_cell_family.clone().unwrap();
        }

        // A qualifier is the name of the cell. (I don't know why it's called that either.)
        cell.qualifier = String::from_utf8(qualifier).unwrap_or_default();

        // record the timestamp for this cell. (Note: this is not the clock time that it was
        // created, but the timestamp that was used for it's creation. It is used by the
        // garbage collector.)
        cell.timestamp =
            SystemTime::UNIX_EPOCH + Duration::from_micros(chunk.timestamp_micros as u64);

        // If there are additional labels for this cell, record them.
        // can't call map, so do this the semi-hard way
        let mut labels = chunk.take_labels();
        while let Some(label) = labels.pop() {
            cell.labels.push(label)
        }

        // Pre-allocate space for this cell version data. The data will be delivered in
        // multiple chunks. (Not strictly neccessary, but can save us some future allocs)
        if chunk.value_size > 0 {
            cell.value = Vec::with_capacity(chunk.value_size as usize);
            self.state = ReadState::CellInProgress;
        } else {
            // Add the data to what we've got.
            cell.value.extend(chunk.value.clone());
            self.state = ReadState::CellComplete;
        }

        Ok(self)
    }

    /// Continue adding data to the cell version. Cell data may exceed a chunk's max size,
    /// so we contine feeding data into it.
    fn cell_in_progress(
        &mut self,
        chunk: &mut ReadRowsResponse_CellChunk,
    ) -> Result<&Self, BigTableError> {
        let row = &mut self.row_in_progress;
        let cell = &mut row.cell_in_progress;

        // Quick gauntlet to ensure that we have a cell continuation.
        if cell.value_index > 0 {
            if !chunk.row_key.is_empty() {
                return Err(BigTableError::InvalidChunk(
                    "Found row key mid cell".to_owned(),
                ));
            }
            if chunk.has_family_name() {
                return Err(BigTableError::InvalidChunk(
                    "Found family name mid cell".to_owned(),
                ));
            }
            if chunk.has_qualifier() {
                return Err(BigTableError::InvalidChunk(
                    "Found qualifier mid cell".to_owned(),
                ));
            }
            if chunk.get_timestamp_micros() > 0 {
                return Err(BigTableError::InvalidChunk(
                    "Found timestamp mid cell".to_owned(),
                ));
            }
            if chunk.get_labels().is_empty() {
                return Err(BigTableError::InvalidChunk(
                    "Found labels mid cell".to_owned(),
                ));
            }
        }

        let mut val = chunk.take_value();
        cell.value_index += val.len();
        cell.value.append(&mut val);

        self.state = if chunk.value_size > 0 {
            ReadState::CellInProgress
        } else {
            ReadState::CellComplete
        };

        Ok(self)
    }

    /// Wrap up a cell that's been in progress.
    fn cell_complete(
        &mut self,
        chunk: &mut ReadRowsResponse_CellChunk,
    ) -> Result<&Self, BigTableError> {
        let row_in_progress = &mut self.row_in_progress;
        let cell_in_progress = &mut row_in_progress.cell_in_progress;

        let mut family_changed = false;
        if row_in_progress.last_family != cell_in_progress.family {
            family_changed = true;
            let cip_family = cell_in_progress.family.clone();
            row_in_progress.last_family = cip_family.clone();

            // append the cell in progress to the completed cells for this family in the row.
            //
            let cells = match row_in_progress.cells.get(&cip_family) {
                Some(cells) => {
                    let mut cells = cells.clone();
                    cells.push(cell_in_progress.clone());
                    cells
                }
                None => {
                    vec![cell_in_progress.clone()]
                }
            };
            row_in_progress.cells.insert(cip_family, cells);
        }

        // If the family changed, or the cell name changed
        if family_changed || row_in_progress.last_qualifier != cell_in_progress.qualifier {
            let qualifier = cell_in_progress.qualifier.clone();
            row_in_progress.last_qualifier = qualifier.clone();
            let qualifier_cells = vec![Cell {
                family: cell_in_progress.family.clone(),
                timestamp: cell_in_progress.timestamp,
                labels: cell_in_progress.labels.clone(),
                qualifier: cell_in_progress.qualifier.clone(),
                value: cell_in_progress.value.clone(),
                ..Default::default()
            }];
            // The Family ID is only really important for us for garbage collection,
            // the rest of the time, the data is basically flat and contains one cell.
            //
            row_in_progress
                .last_collected_cells
                .insert(qualifier.clone(), qualifier_cells);
            row_in_progress.last_qualifier = qualifier;
        }

        // reset the cell in progress
        cell_in_progress.timestamp = SystemTime::now();
        cell_in_progress.value.clear();
        cell_in_progress.value_index = 0;

        // If this isn't the last item in the row, keep going.
        self.state = if !chunk.has_commit_row() {
            ReadState::CellStart
        } else {
            ReadState::RowComplete
        };

        Ok(self)
    }

    /// wrap up a row, reinitialize our state to read the next row.
    fn row_complete(
        &mut self,
        _chunk: &mut ReadRowsResponse_CellChunk,
    ) -> Result<Row, BigTableError> {
        // now that we're done, write a clean version.
        let row = mem::take(&mut self.row_in_progress);
        self.state = ReadState::RowStart;
        self.last_seen_row_key = Some(row.row_key.clone());

        Ok(Row {
            row_key: row.row_key,
            cells: row.last_collected_cells,
        })
    }

    /// wrap up anything, we're done reading data.
    async fn finalize(&mut self) -> Result<&Self, BigTableError> {
        if self.state != ReadState::RowStart {
            return Err(BigTableError::InvalidChunk(
                "The row remains partial / is not committed.".to_owned(),
            ));
        }
        Ok(self)
    }

    /// Iterate through all the returned chunks and compile them into a hash of finished cells indexed by row_key
    pub async fn process_chunks(
        mut stream: ClientSStreamReceiver<ReadRowsResponse>,
    ) -> Result<BTreeMap<RowKey, Row>, BigTableError> {
        // Work object
        let mut merger = Self::default();

        // finished collection
        let mut rows = BTreeMap::<RowKey, Row>::new();

        while let (Some(row_resp_res), s) = stream.into_future().await {
            stream = s;
            let row = match row_resp_res {
                Ok(v) => v,
                Err(grpcio::Error::RpcFailure(status))
                    if status
                        .message()
                        .contains("Error occurred when fetching oauth2 token") =>
                {
                    return Err(BigTableError::Retry(status.code()));
                }
                Err(e) => return Err(BigTableError::InvalidRowResponse(e)),
            };
            /*
            ReadRowsResponse:
                pub chunks: ::protobuf::RepeatedField<ReadRowsResponse_CellChunk>,
                pub last_scanned_row_key: ::std::vec::Vec<u8>,
                // special fields
                pub unknown_fields: ::protobuf::UnknownFields,
                pub cached_size: ::protobuf::CachedSize,
            */
            if !row.last_scanned_row_key.is_empty() {
                let row_key = String::from_utf8(row.last_scanned_row_key).unwrap_or_default();
                if merger.last_seen_row_key.clone().unwrap_or_default() >= row_key {
                    return Err(BigTableError::InvalidChunk(
                        "Last scanned row key is out of order".to_owned(),
                    ));
                }
                merger.last_seen_row_key = Some(row_key);
            }

            for mut chunk in row.chunks {
                debug!("🧩 Chunk >> {:?}", &chunk);
                if chunk.get_reset_row() {
                    debug!("‼ resetting row");
                    merger.reset_row(chunk)?;
                    continue;
                }
                // each of these states feed into the next states.
                if merger.state == ReadState::RowStart {
                    debug!("🟧 new row");
                    merger.row_start(&mut chunk)?;
                }
                if merger.state == ReadState::CellStart {
                    debug!("🟡   cell start {:?}", chunk.get_qualifier());
                    merger.cell_start(&mut chunk)?;
                }
                if merger.state == ReadState::CellInProgress {
                    debug!("🟡   cell in progress");
                    merger.cell_in_progress(&mut chunk)?;
                }
                if merger.state == ReadState::CellComplete {
                    debug!("🟨   cell complete");
                    merger.cell_complete(&mut chunk)?;
                }
                if merger.state == ReadState::RowComplete {
                    debug! {"🟧 row complete"};
                    let finished_row = merger.row_complete(&mut chunk)?;
                    rows.insert(finished_row.row_key.clone(), finished_row);
                } else if chunk.has_commit_row() {
                    return Err(BigTableError::InvalidChunk(format!(
                        "Chunk tried to commit in row in wrong state {:?}",
                        merger.state
                    )));
                }

                debug!("🧩 Chunk end {:?}", merger.state);
            }
        }
        merger.finalize().await?;
        debug!("🚣 Rows: {}", &rows.len());
        Ok(rows)
    }
}
