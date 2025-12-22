// Rust 1.89.0 flags `StorageType` as unused even with `bigtable` set as a default feature.
#[allow(dead_code)]
#[derive(Clone, Eq, PartialEq, Debug)]
pub(crate) enum StorageType {
    #[cfg(feature = "bigtable")]
    BigTable,
    #[cfg(feature = "postgres")]
    Postgres,
    None,
}

impl Default for StorageType {
    fn default() -> StorageType {
        if cfg!(feature = "bigtable") {
            return StorageType::BigTable
        }
        #[cfg(feature = "postgres")]
        if cfg!(feature = "postgres") {
            return StorageType::Postgres
        }
        StorageType::None
    }
}

impl From<&str> for StorageType {
    fn from(str: &str) -> StorageType {
        match str.to_lowercase().as_str() {
            #[cfg(feature = "bigtable")]
            "bigtable" => StorageType::BigTable,
            #[cfg(feature = "postgres")]
            "postgres" => StorageType::Postgres,
            _ => {
                warn!("Using default StorageType for {str}");
                StorageType::default()
            }
        }
    }
}
