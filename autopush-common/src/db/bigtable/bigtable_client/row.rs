use std::collections::HashMap;
use std::time::SystemTime;

use super::{cell::Cell, error::BigTableError, FamilyId, RowKey};

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

    /// get only the "top" cell value. Ignore other values.
    pub fn get_cell(&mut self, family: &str, column: &str) -> Option<Cell> {
        if let Some(mut cells) = self.get_cells(family, column) {
            return cells.pop();
        }
        None
    }

    pub fn get_families(&mut self) -> Vec<String> {
        self.cells.keys().map(|v| v.to_string()).collect()
    }

    pub fn add_cell(
        &mut self,
        family: &str,
        qualifier: &str,
        value: &Vec<u8>,
        ttl: Option<SystemTime>,
    ) -> Result<Cell, BigTableError> {
        let mut cell = Cell {
            family: family.to_owned(),
            qualifier: qualifier.to_owned(),
            value: value.clone(),
            ..Default::default()
        };
        if let Some(ttl) = ttl {
            cell.timestamp = ttl;
        }
        self.cells
            .insert(family.to_owned(), [cell.clone()].to_vec())
            .ok_or(BigTableError::InvalidRowResponse(
                "Could not insert cell".to_owned(),
            ))?;
        Ok(cell)
    }
}
