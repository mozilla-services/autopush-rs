use std::collections::HashMap;

use crate::db::error::{DbError, DbResult};

use super::{cell::Cell, RowKey};

/// A Bigtable storage row. Bigtable stores by Family ID which isn't
/// very useful for us later, so we overload this structure a bit.
/// When we read data back out of Bigtable, we index cells by
/// the cells Qualifier, which allows us to quickly fetch values out
/// of Row.
/// If this feels dirty to you, you're welcome to create a common trait,
/// a pair of matching structs for WriteRow and ReadRow, and the appropriate
/// From traits to handle conversion.
#[derive(Debug, Default, Clone)]
pub struct Row {
    /// The row's key.
    // This may be any ByteArray value.
    pub row_key: RowKey,
    /// The row's collection of cells, indexed by either the
    /// FamilyID (for write) or Qualifier (for read).
    pub cells: HashMap<String, Vec<Cell>>,
}

impl Row {
    /// Create a new Row w/ the specified row_key
    pub fn new(row_key: RowKey) -> Self {
        Row {
            row_key,
            ..Default::default()
        }
    }

    /// Return all cells for a given column
    pub fn take_cells(&mut self, column: &str) -> Option<Vec<Cell>> {
        self.cells.remove(column)
    }

    /// get only the "top" cell value. Ignore other values.
    pub fn take_cell(&mut self, column: &str) -> Option<Cell> {
        if let Some(mut cells) = self.take_cells(column) {
            return cells.pop();
        }
        None
    }

    /// Like [take_cell] but returns an Error when no cell is present
    pub fn take_required_cell(&mut self, column: &str) -> DbResult<Cell> {
        self.take_cell(column)
            .ok_or_else(|| DbError::Integrity(format!("Expected column: {column}")))
    }

    /// Add cells to a given family
    pub fn add_cells(&mut self, family: &str, cells: Vec<Cell>) -> Option<Vec<Cell>> {
        self.cells.insert(family.to_owned(), cells)
    }
}
