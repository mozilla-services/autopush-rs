use std::time::SystemTime;

use super::{merge::PartialCell, FamilyId, Qualifier};

/// A finished Cell. An individual Cell contains the
/// data. There can be multiple cells for a given
/// rowkey::qualifier.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Cell {
    /// The family identifier string.
    pub family: FamilyId,
    /// Column name
    pub qualifier: Qualifier,
    /// Column data
    pub value: Vec<u8>,
    /// the cell's index if returned in a group or array.
    pub value_index: usize,
    /// "Timestamp" in milliseconds. This value is used by the family
    /// garbage collection rules and may not reflect reality.
    pub timestamp: SystemTime,
    pub labels: Vec<String>, // not sure if these are used?
}

impl Default for Cell {
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

impl From<PartialCell> for Cell {
    fn from(partial: PartialCell) -> Cell {
        Self {
            family: partial.family,
            qualifier: partial.qualifier,
            value: partial.value,
            value_index: partial.value_index,
            timestamp: partial.timestamp,
            labels: partial.labels,
        }
    }
}

/*
/// Returns a list of filled cells for the given family.
/// NOTE: Timestamp, here, means whatever the family GC rules dictate.
/// XXX: this may be useful for testing, may remove later.
pub fn fill_cells(
    family: &str,
    timestamp: SystemTime,
    cell_data: HashMap<Qualifier, Vec<u8>>,
) -> Vec<Cell> {
    let mut cells: Vec<Cell> = Vec::new();
    for (value_index, (qualifier, value)) in cell_data.into_iter().enumerate() {
        cells.push(Cell {
            family: family.to_owned(),
            qualifier,
            timestamp,
            value,
            value_index,
            ..Default::default()
        });
    }
    cells
}
*/
