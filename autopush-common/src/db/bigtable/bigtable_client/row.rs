use std::collections::HashMap;

use super::{cell::Cell, FamilyId, RowKey};

/// A finished row. A row consists of a hash of one or more cells per
/// qualifer (cell name).
#[derive(Debug, Default, Clone)]
pub struct Row {
    /// The row's key.
    // This may be any ByteArray value.
    pub row_key: RowKey,
    /// The row's collection of cells, indexed by the family ID.
    pub cells: HashMap<FamilyId, Vec<Cell>>,
}

impl Row {
    /// Return all cells for a given column
    pub fn take_cells(&mut self, column: &str) -> Option<Vec<Cell>> {
        self.cells.remove_entry(column).map(|(_, cell)| cell)
    }

    /// get only the "top" cell value. Ignore other values.
    pub fn take_cell(&mut self, column: &str) -> Option<Cell> {
        if let Some((_, mut cells)) = self.cells.remove_entry(column) {
            return cells.pop();
        }
        None
    }

    /// Add cells to a given column
    pub fn add_cells(&mut self, column: &str, cells: Vec<Cell>) -> Option<Vec<Cell>> {
        self.cells.insert(column.to_owned(), cells)
    }
}
