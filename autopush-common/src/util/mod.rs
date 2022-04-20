//! Various small utilities accumulated over time for the WebPush server
use std::collections::HashMap;
use std::hash::Hash;

// mod send_all; // kill?
pub mod timing;

// pub use self::send_all::MySendAll;
pub use self::timing::{ms_since_epoch, sec_since_epoch, us_since_epoch};

pub trait InsertOpt<K: Eq + Hash, V> {
    /// Insert an item only if it exists
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>);
}

impl<K: Eq + Hash, V> InsertOpt<K, V> for HashMap<K, V> {
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>) {
        if let Some(value) = value {
            self.insert(key.into(), value.into());
        }
    }
}
