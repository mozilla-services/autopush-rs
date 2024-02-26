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

use grpcio::Metadata;
use serde::{Deserialize, Deserializer};
use std::time::Duration;

use crate::db::bigtable::bigtable_client::MetadataBuilder;
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
    #[serde(deserialize_with = "deserialize_u32_to_duration_opt")]
    pub database_pool_connection_wait_timeout: Option<Duration>,
    /// Max time (in seconds) to wait when creating a database connection
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_u32_to_duration_opt")]
    pub database_pool_connection_create_timeout: Option<Duration>,
    /// Max time (in seconds) to wait when recycling a database connection
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_u32_to_duration_opt")]
    pub database_pool_connection_recycle_timeout: Option<Duration>,
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

impl BigTableDbSettings {
    pub fn metadata(&self) -> Result<Metadata, BigTableError> {
        MetadataBuilder::with_prefix(&self.table_name)
            .routing_param("table_name", &self.table_name)
            .route_to_leader(self.route_to_leader)
            .build()
            .map_err(BigTableError::GRPC)
    }

    pub fn admin_metadata(&self) -> Result<Metadata, BigTableError> {
        // Admin calls use a slightly different routing param and a truncated prefix
        // See https://github.com/googleapis/google-cloud-cpp/issues/190#issuecomment-370520185
        let Some(admin_prefix) = self.table_name.split_once("/tables/").map(|v| v.0) else {
            return Err(BigTableError::Admin(
                "Invalid table name specified".to_owned(),
                None,
            ));
        };
        MetadataBuilder::with_prefix(admin_prefix)
            .routing_param("name", &self.table_name)
            .route_to_leader(self.route_to_leader)
            .build()
            .map_err(BigTableError::GRPC)
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

pub fn deserialize_u32_to_duration_opt<'de, D>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error>
where
    D: Deserializer<'de>,
{
    let maybe_seconds: Option<u32> = Option::deserialize(deserializer)?;
    Ok(maybe_seconds.map(|s| Duration::from_secs(s.into())))
}
