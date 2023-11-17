use async_trait::async_trait;
use mockall::automock;
use uuid::Uuid;

use crate::db::error::DbResult;

#[derive(Clone, Eq, PartialEq, Debug)]
pub(crate) enum StorageType {
    DynamoDB,
    BigTable,
    None,
}

impl Default for StorageType {
    fn default() -> StorageType {
        if cfg!(feature = "dynamodb") {
            StorageType::DynamoDB
        } else if cfg!(feature = "bigtable") {
            StorageType::BigTable
        } else {
            StorageType::None
        }
    }
}

impl StorageType {
    pub fn as_str(&self) -> &'static str {
        match &self {
            StorageType::DynamoDB => "dynamodb",
            StorageType::BigTable => "bigtable",
            StorageType::None => "None",
        }
    }
}

impl From<&str> for StorageType {
    fn from(str: &str) -> StorageType {
        match str.to_lowercase().as_str() {
            "dynamodb" => StorageType::DynamoDB,
            "bigtable" => StorageType::BigTable,
            _ => {
                warn!("Using default StorageType for {str}");
                StorageType::default()
            }
        }
    }
}
