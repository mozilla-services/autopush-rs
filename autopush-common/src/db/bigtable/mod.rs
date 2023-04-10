/// TODO: Build out the BigTable integration based off of the
/// autopush-bt::bittable_client code.
///
///
mod bigtable_client;

pub use bigtable_client::error::BigTableError;
pub use bigtable_client::BigTableClientImpl;

use serde::Deserialize;

use crate::db::error::DbError;

#[derive(Clone, Debug, Deserialize)]
pub struct BigTableDbSettings {
    pub table_name: String,
    pub dsn: String,
    pub router_family: String,
    pub message_family: String,
    pub message_topic_family: String,
}

impl Default for BigTableDbSettings {
    fn default() -> Self {
        Self {
            table_name: "autopush".to_owned(),
            dsn: "NO DSN DEFINED".to_owned(),
            router_family: "router_family".to_owned(),
            message_family: "message_family".to_owned(),
            message_topic_family: "message_topic_family".to_owned(),
        }
    }
}

impl TryFrom<&str> for BigTableDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(setting_string)
            .map_err(|e| DbError::General(format!("Could not parse DdbSettings: {:?}", e)))
    }
}
