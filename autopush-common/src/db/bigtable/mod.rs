/// TODO: Build out the BigTable integration based off of the
/// autopush-bt::bittable_client code.
///
///
mod bigtable_client;

pub use bigtable_client::error::BigTableError;
pub use bigtable_client::BigTableClientImpl;

use serde::Deserialize;

use crate::db::error::DbError;

/// The settings for accessing the BigTable contents.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct BigTableDbSettings {
    /// The Table name matches the GRPC template for table paths.
    /// e.g. `projects/{projectid}/instances/{instanceid}/tables/{tablename}`
    /// *NOTE* There is no leading `/`
    /// By default, this (may?) use the `*` variant which translates to
    /// `projects/*/instances/*/tables/*` which searches all data stored in
    /// bigtable.
    #[serde(default)]
    pub table_name: String,
    #[serde(default)]
    pub router_family: String,
    #[serde(default)]
    pub message_family: String,
    #[serde(default)]
    pub message_topic_family: String,
}

/// NOTE: autopush will not autogenerate these families. They should
/// be created when the table is first provisioned. See
/// [BigTable schema](https://cloud.google.com/bigtable/docs/schema-design)
///
/// BE SURE TO CONFIRM the names of the families. These are not checked on
/// initialization, but will throw errors if not present or incorrectly
/// spelled.
///
impl Default for BigTableDbSettings {
    fn default() -> Self {
        Self {
            table_name: "autopush".to_owned(),
            router_family: "router".to_owned(),
            message_family: "message".to_owned(),
            message_topic_family: "message_topic".to_owned(),
        }
    }
}

impl TryFrom<&str> for BigTableDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        let me: Self = serde_json::from_str(setting_string)
            .map_err(|e| DbError::General(format!("Could not parse DdbSettings: {:?}", e)))?;

        if me.table_name.starts_with('/') {
            return Err(DbError::ConnectionError(
                "Table name path begins with a '/'".to_owned(),
            ));
        };

        Ok(me)
    }
}