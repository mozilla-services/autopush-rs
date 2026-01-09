// Rust 1.89.0 flags `StorageType` as unused even with `bigtable` set as a default feature.
#[allow(dead_code)]
#[derive(Clone, Eq, PartialEq, Debug)]
pub(crate) enum StorageType {
    BigTable,
    Redis,
    None,
}

impl Default for StorageType {
    fn default() -> StorageType {
        if cfg!(feature = "bigtable") {
            StorageType::BigTable
        } else if cfg!(feature = "redis") {
            StorageType::Redis
        } else {
            StorageType::None
        }
    }
}

impl From<&str> for StorageType {
    fn from(str: &str) -> StorageType {
        match str.to_lowercase().as_str() {
            "bigtable" => StorageType::BigTable,
            "redis" => StorageType::Redis,
            _ => {
                warn!("Using default StorageType for {str}");
                StorageType::default()
            }
        }
    }
}
