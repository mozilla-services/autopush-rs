use std::collections::HashMap;

use super::{cell::Cell, Qualifier, RowKey};

/// A finished row. A row consists of a hash of one or more cells per
/// qualifer (cell name).
#[derive(Debug, Default, Clone)]
pub struct Row {
    /// The row's key.
    // This may be any ByteArray value.
    pub row_key: RowKey,
    /// The row's collection of cells, indexed by the family ID.
    pub cells: HashMap<Qualifier, Vec<Cell>>,
}

impl Row {
    pub fn get_cells(&self, family: &str, column: &str) -> Option<Vec<Cell>> {
        let mut result = Vec::<Cell>::new();
        if let Some(family_group) = self.cells.get(family) {
            for cell in family_group {
                if cell.qualifier == column {
                    result.push(cell.clone())
                }
            }
        }
        if !result.is_empty() {
            Some(result)
        } else {
            None
        }
    }
}
