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
use serde::Deserialize;
use std::time::Duration;

use crate::db::bigtable::bigtable_client::MetadataBuilder;
use crate::db::error::DbError;
use crate::util::deserialize_opt_u32_to_duration;

fn retry_default() -> usize {
    bigtable_client::RETRY_COUNT
}

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
    /// Routing replication profile id.
    /// Should be used everywhere we set `table_name` when creating requests
    #[serde(default)]
    pub profile_id: String,
    #[serde(default)]
    pub router_family: String,
    #[serde(default)]
    pub message_family: String,
    #[serde(default)]
    pub message_topic_family: String,
    #[serde(default)]
    pub database_pool_max_size: Option<u32>,
    /// Max time (in seconds) to wait to create a new connection to bigtable
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_create_timeout: Option<Duration>,
    /// Max time (in seconds) to wait for a socket to become available
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_wait_timeout: Option<Duration>,
    /// Max time(in seconds) to recycle a connection
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_recycle_timeout: Option<Duration>,
    /// Max time (in seconds) a connection should live
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_connection_ttl: Option<Duration>,
    /// Max idle time(in seconds) for a connection
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_max_idle: Option<Duration>,
    /// Include route to leader header in metadata
    #[serde(default)]
    pub route_to_leader: bool,
    /// Number of times to retry a GRPC function
    #[serde(default = "retry_default")]
    pub retry_count: usize,
}

// Used by test, but we don't want available for release.
#[allow(clippy::derivable_impls)]
#[cfg(test)]
impl Default for BigTableDbSettings {
    fn default() -> Self {
        Self {
            table_name: Default::default(),
            router_family: Default::default(),
            message_family: Default::default(),
            message_topic_family: Default::default(),
            database_pool_max_size: Default::default(),
            database_pool_create_timeout: Default::default(),
            database_pool_wait_timeout: Default::default(),
            database_pool_recycle_timeout: Default::default(),
            database_pool_connection_ttl: Default::default(),
            database_pool_max_idle: Default::default(),
            route_to_leader: Default::default(),
            retry_count: Default::default(),
            profile_id: Default::default(),
        }
    }
}

impl BigTableDbSettings {
    pub fn metadata(&self) -> Result<Metadata, BigTableError> {
        MetadataBuilder::with_prefix(&self.table_name)
            .routing_param("table_name", &self.table_name)
            .route_to_leader(self.route_to_leader)
            .build()
            .map_err(BigTableError::GRPC)
    }

    // Health may require a different metadata declaration.
    pub fn health_metadata(&self) -> Result<Metadata, BigTableError> {
        self.metadata()
    }

    pub fn admin_metadata(&self) -> Result<Metadata, BigTableError> {
        // Admin calls use a slightly different routing param and a truncated prefix
        // See https://github.com/googleapis/google-cloud-cpp/issues/190#issuecomment-370520185
        let Some(admin_prefix) = self.table_name.split_once("/tables/").map(|v| v.0) else {
            return Err(BigTableError::Config(
                "Invalid table name specified".to_owned(),
            ));
        };
        MetadataBuilder::with_prefix(admin_prefix)
            .routing_param("name", &self.table_name)
            .route_to_leader(self.route_to_leader)
            .build()
            .map_err(BigTableError::GRPC)
    }

    pub fn get_instance_name(&self) -> Result<String, BigTableError> {
        let parts: Vec<&str> = self.table_name.split('/').collect();
        if parts.len() < 4 || parts[0] != "projects" || parts[2] != "instances" {
            return Err(BigTableError::Config(
                "Invalid table name specified. Cannot parse instance".to_owned(),
            ));
        }
        Ok(parts[0..4].join("/"))
    }
}

impl TryFrom<&str> for BigTableDbSettings {
    type Error = DbError;
    fn try_from(setting_string: &str) -> Result<Self, Self::Error> {
        let mut me: Self = serde_json::from_str(setting_string)
            .map_err(|e| DbError::General(format!("Could not parse DdbSettings: {:?}", e)))?;

        if me.table_name.starts_with('/') {
            return Err(DbError::ConnectionError(
                "Table name path begins with a '/'".to_owned(),
            ));
        };

        // specify the default string "default" if it's not specified.
        // There's a small chance that this could be reported as "unspecified", so this
        // removes that confusion.
        if me.profile_id.is_empty() {
            me.profile_id = "default".to_owned();
        }

        Ok(me)
    }
}

mod tests {

    #[test]
    fn test_settings_parse() -> Result<(), crate::db::error::DbError> {
        let settings =
            super::BigTableDbSettings::try_from("{\"database_pool_create_timeout\": 123}")?;
        assert_eq!(
            settings.database_pool_create_timeout,
            Some(std::time::Duration::from_secs(123))
        );
        Ok(())
    }
    #[test]
    fn test_get_instance() -> Result<(), super::BigTableError> {
        let settings = super::BigTableDbSettings {
            table_name: "projects/foo/instances/bar/tables/gorp".to_owned(),
            ..Default::default()
        };
        let res = settings.get_instance_name()?;
        assert_eq!(res.as_str(), "projects/foo/instances/bar");

        let settings = super::BigTableDbSettings {
            table_name: "projects/foo/".to_owned(),
            ..Default::default()
        };
        assert!(settings.get_instance_name().is_err());

        let settings = super::BigTableDbSettings {
            table_name: "protect/foo/instances/bar/tables/gorp".to_owned(),
            ..Default::default()
        };
        assert!(settings.get_instance_name().is_err());

        let settings = super::BigTableDbSettings {
            table_name: "project/foo/instance/bar/tables/gorp".to_owned(),
            ..Default::default()
        };
        assert!(settings.get_instance_name().is_err());

        Ok(())
    }
}
