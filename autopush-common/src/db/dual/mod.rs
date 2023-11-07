//! Dual data store.
//!
//! This uses two data stores, a primary and a secondary, and if a
//! read operation fails for the primary, the secondary is automatically
//! used. All write operations ONLY go to the primary.
//!
//! This requires both the `dynamodb` and `bigtable` features.
//!
use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use cadence::{CountedExt, StatsdClient};
use serde::Deserialize;
use serde_json::from_str;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    routing::DbRouting,
    DbSettings, Notification, User,
};

use super::StorageType;

use crate::db::bigtable::BigTableClientImpl;
use crate::db::dynamodb::DdbClientImpl;

#[derive(Clone)]
pub struct DualClientImpl {
    primary: BigTableClientImpl,
    secondary: DdbClientImpl,
    /// This contains the "top limit" for the accounts to send to "Primary".
    /// The first byte of the UAID is taken and compared to this value. All values equal
    /// to or below this top byte are sent to primary, with the rest going to "Secondary"
    median: Option<u8>,
    _metrics: Arc<StatsdClient>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DualDbSettings {
    #[serde(default)]
    primary: DbSettings,
    #[serde(default)]
    secondary: DbSettings,
    /// This contains the "top limit" for the accounts to send to "Primary".
    /// The first byte of the UAID is taken and compared to this value. All values equal
    /// to or below this top byte are sent to primary, with the rest going to "Secondary"
    /// NOTE: specify as RAW hex string (e.g. `0A` not `0x0A`)
    #[serde(default)]
    median: Option<String>,
}

impl DualClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        // Not really sure we need the dsn here.
        let db_settings: DualDbSettings = from_str(&settings.db_settings).map_err(|e| {
            DbError::General(format!("Could not parse DualDBSettings string {:?}", e))
        })?;
        debug!("settings: {:?}", &db_settings.median);
        if StorageType::from_dsn(&db_settings.primary.dsn) != StorageType::BigTable {
            return Err(DbError::General(
                "Invalid primary DSN specified (must be BigTable type)".to_owned(),
            ));
        }
        if StorageType::from_dsn(&db_settings.secondary.dsn) != StorageType::DynamoDb {
            return Err(DbError::General(
                "Invalid secondary DSN specified (must be DynamoDB type)".to_owned(),
            ));
        }
        // determine which uaids to move based on the first byte of their UAID, which (hopefully)
        // should be sufficiently random based on it being a UUID4.
        let median = if let Some(median) = db_settings.median {
            let median = hex::decode(median).map_err(|e| {
                DbError::General(format!(
                    "Could not parse median string. Please use a valid Hex identifier: {:?}",
                    e,
                ))
            })?[0];
            debug!(
                "⚖ Setting median to {:02} ({})",
                hex::encode([median]),
                &median
            );
            Some(median)
        } else {
            None
        };
        let primary = BigTableClientImpl::new(metrics.clone(), &db_settings.primary)?;
        let secondary = DdbClientImpl::new(metrics.clone(), &db_settings.secondary)?;
        Ok(Self {
            primary,
            secondary: secondary.clone(),
            median,
            _metrics: metrics,
        })
    }
}

/// Wrapper functions to allow us to change which data store system actually manages the
/// user allocation routing table.
impl DualClientImpl {
    /// Route and assign a user to the appropriate back end based on the defined
    /// allowance
    async fn allot<'a>(&'a self, uaid: &Uuid) -> DbResult<Box<&'a dyn DbClient>> {
        if let Some(median) = self.median {
            if uaid.as_bytes()[0] <= median {
                info!("⚖ Routing user to Bigtable");
                self.assign(uaid).await?;
                // TODO: Better label for this? It's migration so it would appear as
                // `auto[endpoint|connect].migrate` Maybe that's enough? Do we need
                // tags?
                let _ = self
                    ._metrics
                    .incr("migrate.assign")
                    .map_err(|e| DbError::General(format!("Metrics error {:?}", e)))?;
                Ok(Box::new(&self.primary))
            } else {
                Ok(Box::new(&self.secondary))
            }
        } else {
            Ok(self.target(uaid).await?.0)
        }
    }

    /// Return DynamoDB as the default target storage system, otherwise use BigTable.
    async fn target<'a>(&'a self, uaid: &Uuid) -> DbResult<(Box<&'a dyn DbClient>, bool)> {
        if self.secondary.select(uaid).await? == Some(crate::db::routing::StorageType::BigTable) {
            return Ok((Box::new(&self.primary), true));
        };
        Ok((Box::new(&self.secondary), false))
    }

    /// Assign the user to the new BigTable storage system.
    async fn assign(&self, uaid: &Uuid) -> DbResult<()> {
        self.secondary
            .assign(uaid, super::routing::StorageType::BigTable)
            .await
    }
}

#[async_trait]
impl DbClient for DualClientImpl {
    async fn add_user(&self, user: &User) -> DbResult<()> {
        let target: Box<&dyn DbClient> = self.allot(&user.uaid).await?;
        target.add_user(user).await
    }

    async fn update_user(&self, user: &User) -> DbResult<bool> {
        //  If the UAID is in the allowance, move them to the new data store
        let target: Box<&dyn DbClient> = self.allot(&user.uaid).await?;
        target.update_user(user).await
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let (target, is_primary) = self.target(uaid).await?;
        match target.get_user(uaid).await {
            Ok(Some(user)) => Ok(Some(user)),
            Ok(None) => {
                if is_primary {
                    // The user wasn't in the current primary, so fetch them from the secondary.
                    if let Ok(Some(user)) = self.secondary.get_user(uaid).await {
                        // copy the user record over to the new data store.
                        info!("⚖ Found user record in secondary, moving to primary");
                        self.primary.add_user(&user).await?;
                        let _ = self
                            ._metrics
                            .incr("migrate.moved")
                            .map_err(|e| DbError::General(format!("Metrics error {:?}", e)));
                        return Ok(Some(user));
                    }
                }
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let (target, is_primary) = self.target(uaid).await?;
        let result = target.remove_user(uaid).await?;
        if is_primary {
            // try removing the user from the old store, just in case.
            // leaving them could cause false reporting later.
            let _ = self.secondary.remove_user(uaid).await;
        }
        Ok(result)
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        let (target, _) = self.target(uaid).await?;
        target.add_channel(uaid, channel_id).await
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let (target, is_primary) = self.target(uaid).await?;
        let mut channels = target.get_channels(uaid).await?;
        // check to see if we need to copy over channels from the secondary
        if channels.is_empty() && is_primary {
            channels = self.secondary.get_channels(uaid).await?;
        }
        Ok(channels)
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let (target, is_primary) = self.target(uaid).await?;
        let result = target.remove_channel(uaid, channel_id).await?;
        if is_primary {
            let _ = self.secondary.remove_channel(uaid, channel_id).await?;
        }
        Ok(result)
    }

    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        let (target, is_primary) = self.target(uaid).await?;
        let result = target.remove_node_id(uaid, node_id, connected_at).await?;
        if is_primary {
            let _ = self
                .secondary
                .remove_node_id(uaid, node_id, connected_at)
                .await?;
        }
        Ok(result)
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let target: Box<&dyn DbClient> = self.allot(uaid).await?;
        target.save_message(uaid, message).await
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        let (target, is_primary) = self.target(uaid).await?;
        let result = target.remove_message(uaid, sort_key).await?;
        if is_primary {
            let _ = self.primary.remove_message(uaid, sort_key).await?;
        }
        Ok(result)
    }

    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let (target, is_primary) = self.target(uaid).await?;
        let result = target.fetch_topic_messages(uaid, limit).await?;
        if result.messages.is_empty() && is_primary {
            return self.secondary.fetch_topic_messages(uaid, limit).await;
        }
        return Ok(result);
    }

    async fn fetch_timestamp_messages(
        &self,
        uaid: &Uuid,
        timestamp: Option<u64>,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let (target, is_primary) = self.target(uaid).await?;
        let result = target
            .fetch_timestamp_messages(uaid, timestamp, limit)
            .await?;
        if result.messages.is_empty() && is_primary {
            return self
                .secondary
                .fetch_timestamp_messages(uaid, timestamp, limit)
                .await;
        }
        return Ok(result);
    }

    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        let (target, _) = self.target(uaid).await?;
        target.save_messages(uaid, messages).await
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        let (target, _) = self.target(uaid).await?;
        target.increment_storage(uaid, timestamp).await
    }

    async fn health_check(&self) -> DbResult<bool> {
        Ok(self.primary.health_check().await? && self.secondary.health_check().await?)
    }

    async fn router_table_exists(&self) -> DbResult<bool> {
        self.primary.router_table_exists().await
    }

    async fn message_table_exists(&self) -> DbResult<bool> {
        self.primary.message_table_exists().await
    }

    fn rotating_message_table(&self) -> Option<&str> {
        None
    }

    fn box_clone(&self) -> Box<dyn DbClient> {
        Box::new(self.clone())
    }

    fn name(&self) -> String {
        "Dual".to_owned()
    }
}

#[cfg(all(test, feature = "bigtable", feature = "dynamodb"))]
mod test {
    use super::*;
    use cadence::{NopMetricSink, StatsdClient};
    use serde_json::json;
    use std::str::FromStr;

    fn test_args(median: Option<&str>) -> String {
        json!({
            "primary": {
                "dsn": "grpc://bigtable.googleapis.com",  // Note that this is the general endpoint.
                "db_settings": json!({
                    "table_name": "projects/some-project/instances/some-instance/tables/some-table", // Note the full path.
                    "message_family": "messageFamily",
                    "router_family": "routerFamily",
                }).to_string(),
            },
            "secondary": {
                "dsn": "http://localhost:8000/",
                "db_settings": json!({
                    "router_table": "test_router",
                    "message_table": "test_message",
                }).to_string(),
            },
            "median": median.to_owned(),
        })
        .to_string()
    }

    /// This test checks the dual parser, but also serves as a bit of
    /// documentation for how the db_settings argument should be structured
    #[test]
    fn arg_parsing() -> DbResult<()> {
        let arg_str = test_args(None);
        // the output string looks like:
        /*
        "{\"primary\":{\"db_settings\":\"{\\\"message_family\\\":\\\"message\\\",\\\"router_family\\\":\\\"router\\\",\\\"table_name\\\":\\\"projects/some-project/instances/some-instance/tables/some-table\\\"}\",\"dsn\":\"grpc://bigtable.googleapis.com\"},\"secondary\":{\"db_settings\":\"{\\\"message_table\\\":\\\"test_message\\\",\\\"router_table\\\":\\\"test_router\\\"}\",\"dsn\":\"http://localhost:8000/\"}}"
         */
        // some additional escaping may be required to encode this as an environment variable.
        // not all variables are required, feel free to liberally use fall-back defaults.

        // dbg!(&arg_str);

        let dual_settings = DbSettings {
            dsn: Some("dual".to_owned()),
            db_settings: arg_str,
        };
        let metrics = Arc::new(StatsdClient::builder("", NopMetricSink).build());

        // this is the actual test. It should create `dual` without raising an error
        // Specify "BIGTABLE_EMULATOR_HOST" to skip credential check for emulator.
        let dual = DualClientImpl::new(metrics, &dual_settings)?;

        // for sanity sake, make sure the passed message_family isn't the default value.
        assert_eq!(&dual.primary.settings.message_family, "messageFamily");

        Ok(())
    }

    #[actix_rt::test]
    async fn allocation() -> DbResult<()> {
        let arg_str = test_args(Some("0A"));
        let metrics = Arc::new(StatsdClient::builder("", NopMetricSink).build());
        let dual_settings = DbSettings {
            dsn: Some("dual".to_owned()),
            db_settings: arg_str,
        };
        let dual = DualClientImpl::new(metrics, &dual_settings)?;

        // Should be included.
        let low_uaid = Uuid::from_str("04DDDDDD-2040-4b4d-be3d-a340fc2d15a6").unwrap();
        // Should be excluded.
        let hi_uaid = Uuid::from_str("0ADDDDDD-2040-4b4d-be3d-a340fc2d15a6").unwrap();
        let result = dual.allot(&low_uaid).await?;
        assert_eq!(result.name(), dual.primary.name());
        let result = dual.allot(&hi_uaid).await?;
        assert_eq!(result.name(), dual.secondary.name());
        Ok(())
    }
}
