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
mod pool;

pub use bigtable_client::error::BigTableError;
pub use bigtable_client::BigTableClientImpl;

use serde::Deserialize;
use std::time::Duration;

use crate::db::error::DbError;
use crate::util::deserialize_u32_to_duration;

/// The settings for accessing the BigTable contents.
#[derive(Clone, Debug, Deserialize)]
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
    pub database_pool_max_size: Option<u32>,
    /// Max time (in seconds) to wait for a database connection
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_u32_to_duration")]
    pub database_pool_connection_timeout: Duration,
    /// Max time (in seconds) a connection should live
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_u32_to_duration")]
    pub database_pool_connection_ttl: Duration,
    /// Max idle time(in seconds) for a connection
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_u32_to_duration")]
    pub database_pool_max_idle: Duration,
    /// Include route to leader header in metadata
    #[serde(default)]
    pub route_to_leader: bool,
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
