//! Dual data store.
//!
//! This uses two data stores, a primary and a secondary, and if a
//! read operation fails for the primary, the secondary is automatically
//! used. All write operations ONLY go to the primary.
//!
//! This requires both the `dynamodb` and `bigtable` features.
//!
use std::{collections::HashSet, sync::Arc, time::Duration};

use async_trait::async_trait;
use cadence::{CountedExt, StatsdClient, Timed};
use serde::Deserialize;
use serde_json::from_str;
use uuid::Uuid;

use crate::db::{
    bigtable::BigTableClientImpl,
    client::{DbClient, FetchMessageResponse},
    dynamodb::DdbClientImpl,
    error::{DbError, DbResult},
    DbSettings, Notification, User,
};

use super::StorageType;

#[derive(Clone)]
pub struct DualClientImpl {
    /// The primary data store, which will always be Bigtable.
    primary: BigTableClientImpl,
    /// The secondary data store, which will always be DynamoDB.
    secondary: DdbClientImpl,
    /// Write changes to the secondary, including messages and updates
    /// as well as account and channel additions/deletions.
    write_to_secondary: bool,
    /// Hex value to use to specify the first byte of the median offset.
    /// e.g. "0a" will start from include all UUIDs upto and including "0a"
    median: Option<u8>,
    /// Ending octet to use for more distributed account migration
    end_median: Option<u8>,
    metrics: Arc<StatsdClient>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize)]
pub struct DualDbSettings {
    /// The primary data store, which will always be Bigtable.
    primary: DbSettings,
    /// The secondary data store, which will always be DynamoDB.
    secondary: DbSettings,
    /// Write changes to the secondary, including messages and updates
    /// as well as account and channel additions/deletions.
    #[serde(default = "default_true")]
    write_to_secondary: bool,
    /// Hex value to specify the first byte of the median offset.
    /// e.g. "0a" will start from include all UUIDs upto and including "0a"
    #[serde(default)]
    median: Option<String>,
    /// Hex value to specify the last byte of the median offset to include.
    /// this value is "OR"ed withe "median" to produce a more distributed set of
    /// uaids to migrate
    #[serde(default)]
    end_median: Option<String>,
}

impl DualClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        // Not really sure we need the dsn here.
        info!("⚖ Trying: {:?}", settings.db_settings);
        let db_settings: DualDbSettings = from_str(&settings.db_settings).map_err(|e| {
            DbError::General(format!("Could not parse DualDBSettings string {:?}", e))
        })?;
        info!("⚖ {:?}", &db_settings);
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
        // determine which uaids to move based on the last byte of their UAID.
        // This should reduce the hot table problem.
        let end_median = if let Some(end_median) = db_settings.end_median {
            let end_median = hex::decode(end_median).map_err(|e| {
                DbError::General(format!(
                    "Could not parse end_median string. Please use a valid Hex identifier: {:?}",
                    e,
                ))
            })?[0];
            debug!(
                "⚖ Setting end_median to {:02} ({})",
                hex::encode([end_median]),
                &end_median
            );
            Some(end_median)
        } else {
            None
        };

        let primary = BigTableClientImpl::new(metrics.clone(), &db_settings.primary)?;
        let secondary = DdbClientImpl::new(metrics.clone(), &db_settings.secondary)?;
        debug!("⚖ Got primary and secondary");
        metrics
            .incr_with_tags("database.dual.allot")
            .with_tag(
                "median",
                &median.map_or_else(|| "None".to_owned(), |m| m.to_string()),
            )
            .with_tag(
                "end_median",
                &end_median.map_or_else(|| "None".to_owned(), |m| m.to_string()),
            )
            .send();
        Ok(Self {
            primary,
            secondary: secondary.clone(),
            median,
            end_median,
            write_to_secondary: db_settings.write_to_secondary,
            metrics,
        })
    }

    /// Spawn a task to periodically evict idle Bigtable connections
    pub fn spawn_sweeper(&self, interval: Duration) {
        self.primary.spawn_sweeper(interval);
    }
}

/// Wrapper functions to allow us to change which data store system actually manages the
/// user allocation routing table.
impl DualClientImpl {
    fn should_migrate(&self, uaid: &Uuid) -> bool {
        let bytes = uaid.as_bytes();
        let mut result: bool = false;
        if let Some(median) = self.median {
            result |= bytes.first() <= Some(&median);
        };
        if let Some(end_median) = self.end_median {
            result |= bytes.last() <= Some(&end_median);
        }
        result
    }
    /// Route and assign a user to the appropriate back end based on the defined
    /// allowance
    /// Returns the dbclient to use and whether or not it's the primary database.
    async fn allot<'a>(&'a self, uaid: &Uuid) -> DbResult<(Box<&'a dyn DbClient>, bool)> {
        let target: (Box<&'a dyn DbClient>, bool) = if self.should_migrate(uaid) {
            debug!("⚖ Routing user to Bigtable");
            (Box::new(&self.primary), true)
        } else {
            (Box::new(&self.secondary), false)
        };
        debug!("⚖ alloting to {}", target.0.name());
        Ok(target)
    }
}

#[async_trait]
impl DbClient for DualClientImpl {
    async fn add_user(&self, user: &User) -> DbResult<()> {
        let (target, is_primary) = self.allot(&user.uaid).await?;
        debug!("⚖ adding user to {}...", target.name());
        let result = target.add_user(user).await?;
        if is_primary && self.write_to_secondary {
            let _ = self.secondary.add_user(user).await.map_err(|e| {
                error!("⚖ Error: {:?}", e);
                self.metrics
                    .incr_with_tags("database.dual.error")
                    .with_tag("func", "add_user")
                    .send();
                e
            });
        }
        debug!("⚖ User added...");
        Ok(result)
    }

    async fn update_user(&self, user: &mut User) -> DbResult<bool> {
        //  If the UAID is in the allowance, move them to the new data store
        let (target, is_primary) = self.allot(&user.uaid).await?;
        let result = target.update_user(user).await?;
        if is_primary && self.write_to_secondary {
            let _ = self.secondary.update_user(user).await.map_err(|e| {
                error!("⚡ Error: {:?}", e);
                self.metrics
                    .incr_with_tags("database.dual.error")
                    .with_tag("func", "update_user")
                    .send();
                e
            });
        }
        Ok(result)
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        let (target, is_primary) = self.allot(uaid).await?;
        match target.get_user(uaid).await {
            Ok(Some(user)) => Ok(Some(user)),
            Ok(None) => {
                if is_primary {
                    // The user wasn't in the current primary, so fetch them from the secondary.
                    let start = std::time::Instant::now();
                    if let Ok(Some(mut user)) = self.secondary.get_user(uaid).await {
                        // copy the user record over to the new data store.
                        debug!("⚖ Found user record in secondary, moving to primary");
                        // Users read from DynamoDB lack a version field needed
                        // for Bigtable
                        debug_assert!(user.version.is_none());
                        user.version = Some(Uuid::new_v4());
                        if let Err(e) = self.primary.add_user(&user).await {
                            if !matches!(e, DbError::Conditional) {
                                return Err(e);
                            }
                            // User is being migrated underneath us.  Try
                            // fetching the record from primary again, and back
                            // off if still not there.
                            let user = self.primary.get_user(uaid).await?;
                            // Possibly a higher number of these occur than
                            // expected, so sanity check that a user now exists
                            self.metrics
                                .incr_with_tags("database.already_migrated")
                                .with_tag("exists", &user.is_some().to_string())
                                .send();
                            if user.is_none() {
                                return Err(DbError::Backoff("Move in progress".to_owned()));
                            };
                            return Ok(user);
                        };
                        self.metrics.incr_with_tags("database.migrate").send();
                        let channels = self.secondary.get_channels(uaid).await?;
                        if !channels.is_empty() {
                            // NOTE: add_channels doesn't write a new version:
                            // user.version is still valid
                            self.primary.add_channels(uaid, channels).await?;
                        }
                        self.metrics
                            .time_with_tags(
                                "database.migrate.time",
                                (std::time::Instant::now() - start).as_millis() as u64,
                            )
                            .send();
                        return Ok(Some(user));
                    }
                }
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let (target, is_primary) = self.allot(uaid).await?;
        let result = target.remove_user(uaid).await?;
        if is_primary {
            // try removing the user from the old store, just in case.
            // leaving them could cause false reporting later.
            let _ = self.secondary.remove_user(uaid).await.map_err(|e| {
                debug!("⚖ Secondary remove_user error {:?}", e);
                self.metrics
                    .incr_with_tags("database.dual.error")
                    .with_tag("func", "remove_user")
                    .send();
                e
            });
        }
        Ok(result)
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        debug!("⚖ getting target");
        let (target, is_primary) = self.allot(uaid).await?;
        debug!("⚖ Adding channel to {}", target.name());
        let result = target.add_channel(uaid, channel_id).await;
        if is_primary && self.write_to_secondary {
            let _ = self
                .secondary
                .add_channel(uaid, channel_id)
                .await
                .map_err(|e| {
                    self.metrics
                        .incr_with_tags("database.dual.error")
                        .with_tag("func", "add_channel")
                        .send();
                    e
                });
        }
        result
    }

    async fn add_channels(&self, uaid: &Uuid, channels: HashSet<Uuid>) -> DbResult<()> {
        let (target, is_primary) = self.allot(uaid).await?;
        let result = target.add_channels(uaid, channels.clone()).await;
        if is_primary && self.write_to_secondary {
            let _ = self
                .secondary
                .add_channels(uaid, channels)
                .await
                .map_err(|e| {
                    self.metrics
                        .incr_with_tags("database.dual.error")
                        .with_tag("func", "add_channels")
                        .send();
                    e
                });
        }
        result
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let (target, _is_primary) = self.allot(uaid).await?;
        target.get_channels(uaid).await
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let (target, is_primary) = self.allot(uaid).await?;
        let result = target.remove_channel(uaid, channel_id).await?;
        // Always remove the channel
        if is_primary {
            let _ = self
                .secondary
                .remove_channel(uaid, channel_id)
                .await
                .map_err(|e| {
                    debug!("⚖ Secondary remove_channel error: {:?}", e);
                    self.metrics
                        .incr_with_tags("database.dual.error")
                        .with_tag("func", "remove_channel")
                        .send();

                    e
                });
        }
        Ok(result)
    }

    async fn remove_node_id(
        &self,
        uaid: &Uuid,
        node_id: &str,
        connected_at: u64,
        version: &Option<Uuid>,
    ) -> DbResult<bool> {
        let (target, is_primary) = self.allot(uaid).await?;
        let mut result = target
            .remove_node_id(uaid, node_id, connected_at, version)
            .await?;
        // Always remove the node_id.
        if is_primary {
            result = self
                .secondary
                .remove_node_id(uaid, node_id, connected_at, version)
                .await
                .unwrap_or_else(|e| {
                    debug!("⚖ Secondary remove_node_id error: {:?}", e);
                    self.metrics
                        .incr_with_tags("database.dual.error")
                        .with_tag("func", "remove_node_id")
                        .send();

                    false
                })
                || result;
        }
        Ok(result)
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        let (target, is_primary) = self.allot(uaid).await?;
        if is_primary && self.write_to_secondary {
            let _ = self.secondary.save_message(uaid, message.clone()).await?;
        }
        target.save_message(uaid, message).await
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        let (target, is_primary) = self.allot(uaid).await?;
        let result = target.remove_message(uaid, sort_key).await?;
        // Always remove the message
        if is_primary {
            // this will be increasingly chatty as we wind down dynamodb.
            let _ = self
                .secondary
                .remove_message(uaid, sort_key)
                .await
                .map_err(|e| {
                    debug!("⚖ Secondary remove_message error: {:?}", e);
                    self.metrics
                        .incr_with_tags("database.dual.error")
                        .with_tag("func", "remove_message")
                        .send();
                    e
                });
        }
        Ok(result)
    }

    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let (target, is_primary) = self.allot(uaid).await?;
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
        let (target, is_primary) = self.allot(uaid).await?;
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
        let (target, is_primary) = self.allot(uaid).await?;
        if is_primary && self.write_to_secondary {
            let _ = self.secondary.save_messages(uaid, messages.clone()).await?;
        }
        target.save_messages(uaid, messages).await
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        let (target, is_primary) = self.allot(uaid).await?;
        if is_primary {
            let _ = self
                .secondary
                .increment_storage(uaid, timestamp)
                .await
                .map_err(|e| {
                    debug!("⚖ Secondary increment_storage error: {:?}", e);
                    self.metrics
                        .incr_with_tags("database.dual.error")
                        .with_tag("func", "increment_storage")
                        .send();
                    e
                });
        }
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

    fn pool_status(&self) -> Option<deadpool::Status> {
        self.primary.pool_status()
    }
}

#[cfg(all(test, feature = "bigtable", feature = "dynamodb"))]
mod test {
    use super::*;
    use cadence::{NopMetricSink, StatsdClient};
    use serde_json::json;
    use std::str::FromStr;

    fn test_args(median: Option<&str>, end_median: Option<&str>) -> String {
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
            "end_median": end_median.to_owned(),
            "write_to_secondary": false,
        })
        .to_string()
    }

    /// This test checks the dual parser, but also serves as a bit of
    /// documentation for how the db_settings argument should be structured
    #[test]
    fn arg_parsing() -> DbResult<()> {
        let arg_str = test_args(None, None);
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
        let arg_str = test_args(Some("0A"), Some("88"));
        let metrics = Arc::new(StatsdClient::builder("", NopMetricSink).build());
        let dual_settings = DbSettings {
            dsn: Some("dual".to_owned()),
            db_settings: arg_str,
        };
        let dual = DualClientImpl::new(metrics, &dual_settings)?;

        // Should be included (Note: high end median)
        let low_uaid = Uuid::from_str("04DDDDDD-1234-1234-1234-0000000000CC").unwrap();
        let (result, is_primary) = dual.allot(&low_uaid).await?;
        assert_eq!(result.name(), dual.primary.name());
        assert!(is_primary);

        // Should be excluded (Note: high end_median)
        let hi_uaid = Uuid::from_str("0BDDDDDD-1234-1234-1234-0000000000CC").unwrap();
        let (result, is_primary) = dual.allot(&hi_uaid).await?;
        assert_eq!(result.name(), dual.secondary.name());
        assert!(!is_primary);

        // Should be included (Note: high median with low end median)
        let hi_end_uaid = Uuid::from_str("0BDDDDDD-1234-1234-1234-000000000080").unwrap();
        let (result, is_primary) = dual.allot(&hi_end_uaid).await?;
        assert_eq!(result.name(), dual.primary.name());
        assert!(is_primary);

        Ok(())
    }
}
