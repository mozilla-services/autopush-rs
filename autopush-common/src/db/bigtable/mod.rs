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

pub use bigtable_client::BigTableClientImpl;
pub use bigtable_client::error::BigTableError;

use serde::Deserialize;
use std::time::Duration;
use tonic::metadata::MetadataMap;

use crate::db::bigtable::bigtable_client::MetadataBuilder;
use crate::db::error::DbError;
use crate::util::{deserialize_opt_u32_to_duration, deserialize_u32_to_duration};

const DEFAULT_GRPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_GRPC_POINT_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_GRPC_SCAN_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_GRPC_POINT_TOTAL_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_GRPC_SCAN_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
const DEFAULT_GRPC_CHANNEL_REFRESH_MIN: Duration = Duration::from_secs(60);
const DEFAULT_GRPC_CHANNEL_REFRESH_MAX: Duration = Duration::from_secs(180);

fn grpc_connect_timeout_default() -> Duration {
    DEFAULT_GRPC_CONNECT_TIMEOUT
}

fn grpc_point_attempt_timeout_default() -> Duration {
    DEFAULT_GRPC_POINT_ATTEMPT_TIMEOUT
}

fn grpc_scan_attempt_timeout_default() -> Duration {
    DEFAULT_GRPC_SCAN_ATTEMPT_TIMEOUT
}

fn grpc_point_total_timeout_default() -> Duration {
    DEFAULT_GRPC_POINT_TOTAL_TIMEOUT
}

fn grpc_scan_total_timeout_default() -> Duration {
    DEFAULT_GRPC_SCAN_TOTAL_TIMEOUT
}

fn grpc_channel_refresh_min_default() -> Duration {
    DEFAULT_GRPC_CHANNEL_REFRESH_MIN
}

fn grpc_channel_refresh_max_default() -> Duration {
    DEFAULT_GRPC_CHANNEL_REFRESH_MAX
}

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
    pub app_profile_id: String,
    #[serde(default)]
    pub router_family: String,
    #[serde(default)]
    pub message_family: String,
    #[serde(default)]
    pub message_topic_family: String,
    #[serde(default)]
    pub database_pool_max_size: Option<u32>,
    /// Number of shared tonic channels used for Bigtable RPCs. Defaults to two;
    /// tune this from observed per-process concurrency and queueing rather than
    /// from the maximum logical operation-pool size.
    #[serde(default)]
    pub grpc_channel_count: Option<u32>,
    /// Max time (in seconds) to create a pooled client handle.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_create_timeout: Option<Duration>,
    /// Max time (in seconds) to wait for a logical operation slot.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_wait_timeout: Option<Duration>,
    /// Max time (in seconds) to recycle a client handle.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_recycle_timeout: Option<Duration>,
    /// Max time (in seconds) a pooled client handle should live. Tonic channel
    /// lifetime and reconnection are managed independently.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_connection_ttl: Option<Duration>,
    /// Max idle time (in seconds) for a pooled client handle.
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub database_pool_max_idle: Option<Duration>,
    /// Max time (in seconds) for DNS, TCP, and TLS connection establishment.
    #[serde(
        default = "grpc_connect_timeout_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_connect_timeout: Duration,
    /// Per-attempt deadline (in seconds) for point reads and writes.
    #[serde(
        default = "grpc_point_attempt_timeout_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_point_attempt_timeout: Duration,
    /// Per-attempt deadline (in seconds) for message range scans.
    #[serde(
        default = "grpc_scan_attempt_timeout_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_scan_attempt_timeout: Duration,
    /// End-to-end retry budget (in seconds) for point reads and writes.
    #[serde(
        default = "grpc_point_total_timeout_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_point_total_timeout: Duration,
    /// End-to-end retry budget (in seconds) for message range scans.
    #[serde(
        default = "grpc_scan_total_timeout_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_scan_total_timeout: Duration,
    /// Minimum randomized period (in seconds) before eagerly replacing each
    /// Bigtable channel. Refresh is disabled for the emulator.
    #[serde(
        default = "grpc_channel_refresh_min_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_channel_refresh_min: Duration,
    /// Maximum randomized period (in seconds) before eagerly replacing each
    /// Bigtable channel.
    #[serde(
        default = "grpc_channel_refresh_max_default",
        deserialize_with = "deserialize_u32_to_duration"
    )]
    pub grpc_channel_refresh_max: Duration,
    /// Include route to leader header in metadata
    #[serde(default)]
    pub route_to_leader: bool,
    /// Number of retries after the initial gRPC data operation. Defaults to
    /// two. Health checks use this same configured value.
    #[serde(default = "retry_default")]
    pub retry_count: usize,
    /// Max lifetime (in seconds) for a router entry
    #[serde(default)]
    #[serde(deserialize_with = "deserialize_opt_u32_to_duration")]
    pub max_router_ttl: Option<Duration>,
}

// Used by test, but we don't want available for release.
#[allow(clippy::derivable_impls)]
#[cfg(test)]
impl Default for BigTableDbSettings {
    fn default() -> Self {
        use crate::MAX_ROUTER_TTL_SECS;

        Self {
            table_name: Default::default(),
            router_family: Default::default(),
            message_family: Default::default(),
            message_topic_family: Default::default(),
            database_pool_max_size: Default::default(),
            grpc_channel_count: Default::default(),
            database_pool_create_timeout: Default::default(),
            database_pool_wait_timeout: Default::default(),
            database_pool_recycle_timeout: Default::default(),
            database_pool_connection_ttl: Default::default(),
            database_pool_max_idle: Default::default(),
            grpc_connect_timeout: grpc_connect_timeout_default(),
            grpc_point_attempt_timeout: grpc_point_attempt_timeout_default(),
            grpc_scan_attempt_timeout: grpc_scan_attempt_timeout_default(),
            grpc_point_total_timeout: grpc_point_total_timeout_default(),
            grpc_scan_total_timeout: grpc_scan_total_timeout_default(),
            grpc_channel_refresh_min: grpc_channel_refresh_min_default(),
            grpc_channel_refresh_max: grpc_channel_refresh_max_default(),
            route_to_leader: Default::default(),
            retry_count: Default::default(),
            app_profile_id: Default::default(),
            max_router_ttl: Some(Duration::from_secs(MAX_ROUTER_TTL_SECS)),
        }
    }
}

impl BigTableDbSettings {
    pub fn metadata(&self) -> Result<MetadataMap, BigTableError> {
        MetadataBuilder::with_prefix(&self.table_name)
            .routing_param("table_name", &self.table_name)
            .route_to_leader(self.route_to_leader)
            .build()
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
            .map_err(|e| DbError::General(format!("Could not parse DdbSettings: {e:?}")))?;

        if me.table_name.starts_with('/') {
            return Err(DbError::ConnectionError(
                "Table name path begins with a '/'".to_owned(),
            ));
        };

        if me.grpc_channel_count == Some(0) {
            return Err(DbError::ConnectionError(
                "grpc_channel_count must be greater than zero".to_owned(),
            ));
        }

        let nonzero_durations = [
            ("grpc_connect_timeout", me.grpc_connect_timeout),
            ("grpc_point_attempt_timeout", me.grpc_point_attempt_timeout),
            ("grpc_scan_attempt_timeout", me.grpc_scan_attempt_timeout),
            ("grpc_point_total_timeout", me.grpc_point_total_timeout),
            ("grpc_scan_total_timeout", me.grpc_scan_total_timeout),
            ("grpc_channel_refresh_min", me.grpc_channel_refresh_min),
            ("grpc_channel_refresh_max", me.grpc_channel_refresh_max),
        ];
        if let Some((name, _)) = nonzero_durations
            .into_iter()
            .find(|(_, duration)| duration.is_zero())
        {
            return Err(DbError::ConnectionError(format!(
                "{name} must be greater than zero"
            )));
        }
        if me.grpc_point_attempt_timeout > me.grpc_point_total_timeout {
            return Err(DbError::ConnectionError(
                "grpc_point_attempt_timeout must not exceed grpc_point_total_timeout".to_owned(),
            ));
        }
        if me.grpc_scan_attempt_timeout > me.grpc_scan_total_timeout {
            return Err(DbError::ConnectionError(
                "grpc_scan_attempt_timeout must not exceed grpc_scan_total_timeout".to_owned(),
            ));
        }
        if me.grpc_channel_refresh_min > me.grpc_channel_refresh_max {
            return Err(DbError::ConnectionError(
                "grpc_channel_refresh_min must not exceed grpc_channel_refresh_max".to_owned(),
            ));
        }

        // specify the default string "default" if it's not specified.
        // There's a small chance that this could be reported as "unspecified", so this
        // removes that confusion.
        if me.app_profile_id.is_empty() {
            "default".clone_into(&mut me.app_profile_id);
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
        assert_eq!(settings.retry_count, 2);
        assert_eq!(settings.grpc_channel_count, None);
        assert_eq!(
            settings.grpc_connect_timeout,
            std::time::Duration::from_secs(5)
        );
        assert_eq!(
            settings.grpc_point_attempt_timeout,
            std::time::Duration::from_secs(5)
        );
        assert_eq!(
            settings.grpc_scan_attempt_timeout,
            std::time::Duration::from_secs(20)
        );
        assert_eq!(
            settings.grpc_point_total_timeout,
            std::time::Duration::from_secs(15)
        );
        assert_eq!(
            settings.grpc_scan_total_timeout,
            std::time::Duration::from_secs(30)
        );
        Ok(())
    }

    #[test]
    fn test_zero_grpc_channel_count_is_rejected() {
        let result = super::BigTableDbSettings::try_from("{\"grpc_channel_count\": 0}");
        assert!(result.is_err());
    }

    #[test]
    fn test_invalid_grpc_timeouts_are_rejected() {
        assert!(
            super::BigTableDbSettings::try_from(
                "{\"grpc_point_attempt_timeout\": 6, \"grpc_point_total_timeout\": 5}"
            )
            .is_err()
        );
        assert!(
            super::BigTableDbSettings::try_from(
                "{\"grpc_channel_refresh_min\": 181, \"grpc_channel_refresh_max\": 180}"
            )
            .is_err()
        );
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
