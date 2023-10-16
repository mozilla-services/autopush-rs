/// This uses Google Cloud Platform (GCP) Bigtable as a storage and management
/// system for Autopush Notifications and Routing information.
///
/// Bigtable has a single index key, and uses "cell family" designators to
/// perform garbage collection.
///
/// Keys for the data are
/// `{uaid}` - the meta data record around a given UAID record
/// `{uaid}#{channelid}` - the meta record for a channel associated with a
///     UAID
/// `{uaid}#{channelid}#{sortkey_timestamp}` - a message record for a UAID
///     and channel
///
/// Bigtable will automatically sort by the primary key. This schema uses
/// regular expression lookups in order to do things like return the channels
/// associated with a given UAID, fetch the appropriate topic messages, and
/// other common functions. Please refer to the Bigtable documentation
/// for how to create these keys, since they must be inclusive. Partial
/// key matches will not return data. (e.g `/foo/` will not match `foobar`,
/// but `/foo.*/` will)
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
    #[serde(default)]
    pub db_routing_table: Option<String>,
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
            db_routing_table: None,
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
