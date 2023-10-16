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
/// The following uses a common "dbroute" table to determine where the main storage is for a given
/// UAID. NOTE: There Must Be Only One `dbroute` Table, but where it lives can be irrelevant.
/// (e.g. it could live on AWS or GCP, or on some other data store that is accessible to both
/// `autoconnect` and `autoendpoint`)
#[automock]
#[async_trait]
pub(crate) trait DbRouting {
    /// Connect (and initialize if not present) the DbRouting table.
    async fn connect(&self) -> DbResult<()>;

    /// Select the data source for the UAID, returning None if no entry found.
    async fn select(&self, uaid: &Uuid) -> DbResult<Option<StorageType>>;

    /// Set the data storage type for the UAID
    async fn assign(&self, uaid: &Uuid, storage_type: StorageType) -> DbResult<()>;

    fn box_clone(&self) -> Box<dyn DbRouting>;
}

impl Clone for Box<dyn DbRouting> {
    fn clone(&self) -> Self {
        self.box_clone()
    }
}
