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
use cadence::StatsdClient;
use serde::Deserialize;
use serde_json::from_str;
use uuid::Uuid;

use crate::db::{
    client::{DbClient, FetchMessageResponse},
    error::{DbError, DbResult},
    DbSettings, Notification, User,
};

use super::StorageType;

use crate::db::bigtable::BigTableClientImpl;
use crate::db::dynamodb::DdbClientImpl;

#[derive(Clone)]
pub struct DualClientImpl {
    primary: BigTableClientImpl,
    secondary: DdbClientImpl,
    write_to_secondary: bool,
    _metrics: Arc<StatsdClient>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DualDbSettings {
    #[serde(default)]
    primary: DbSettings,
    #[serde(default)]
    secondary: DbSettings,
    #[serde(default = "set_true")]
    write_to_secondary: bool,
}

fn set_true() -> bool {
    true
}

impl DualClientImpl {
    pub fn new(metrics: Arc<StatsdClient>, settings: &DbSettings) -> DbResult<Self> {
        // Not really sure we need the dsn here.
        let db_settings: DualDbSettings = from_str(&settings.db_settings).map_err(|e| {
            DbError::General(format!("Could not parse DualDBSettings string {:?}", e))
        })?;
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
        let primary = BigTableClientImpl::new(metrics.clone(), &db_settings.primary)?;
        let secondary = DdbClientImpl::new(metrics.clone(), &db_settings.secondary)?;
        Ok(Self {
            primary,
            secondary,
            _metrics: metrics,
            write_to_secondary: db_settings.write_to_secondary,
        })
    }
}

#[async_trait]
impl DbClient for DualClientImpl {
    async fn add_user(&self, user: &User) -> DbResult<()> {
        if self.write_to_secondary {
            let _ = self.secondary.add_user(user).await?;
        }
        self.primary.add_user(user).await
    }

    async fn update_user(&self, user: &User) -> DbResult<()> {
        if self.write_to_secondary {
            let _ = self.secondary.update_user(user).await?;
        }
        self.primary.update_user(user).await
    }

    async fn get_user(&self, uaid: &Uuid) -> DbResult<Option<User>> {
        match self.primary.get_user(uaid).await {
            Ok(Some(user)) => Ok(Some(user)),
            Ok(None) => {
                if let Ok(Some(user)) = self.secondary.get_user(uaid).await {
                    // copy the user record over to the new data store.
                    self.primary.add_user(&user).await?;
                    return Ok(Some(user));
                }
                Ok(None)
            }
            Err(err) => Err(err),
        }
    }

    async fn remove_user(&self, uaid: &Uuid) -> DbResult<()> {
        let result = self.primary.remove_user(uaid).await?;
        // try removing the user from the old store, just in case.
        // leaving them could cause false reporting later.
        let _ = self.secondary.remove_user(uaid).await;
        Ok(result)
    }

    async fn add_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<()> {
        if self.write_to_secondary {
            let _ = self.secondary.add_channel(uaid, channel_id).await?;
        }
        self.primary.add_channel(uaid, channel_id).await
    }

    async fn get_channels(&self, uaid: &Uuid) -> DbResult<HashSet<Uuid>> {
        let mut channels = self.primary.get_channels(uaid).await?;
        // check to see if we need to copy over channels from the secondary
        if channels.is_empty() {
            channels = self.secondary.get_channels(uaid).await?;
        }
        Ok(channels)
    }

    async fn remove_channel(&self, uaid: &Uuid, channel_id: &Uuid) -> DbResult<bool> {
        let presult = self.primary.remove_channel(uaid, channel_id).await?;
        let sresult = self.secondary.remove_channel(uaid, channel_id).await?;
        Ok(presult | sresult)
    }

    async fn remove_node_id(&self, uaid: &Uuid, node_id: &str, connected_at: u64) -> DbResult<()> {
        let _ = self
            .secondary
            .remove_node_id(uaid, node_id, connected_at)
            .await;
        self.primary
            .remove_node_id(uaid, node_id, connected_at)
            .await
    }

    async fn save_message(&self, uaid: &Uuid, message: Notification) -> DbResult<()> {
        if self.write_to_secondary {
            let _ = self.secondary.save_message(uaid, message.clone()).await?;
        }
        self.primary.save_message(uaid, message).await
    }

    async fn remove_message(&self, uaid: &Uuid, sort_key: &str) -> DbResult<()> {
        let _ = self.secondary.remove_message(uaid, sort_key).await;
        self.primary.remove_message(uaid, sort_key).await
    }

    async fn fetch_topic_messages(
        &self,
        uaid: &Uuid,
        limit: usize,
    ) -> DbResult<FetchMessageResponse> {
        let result = self.primary.fetch_topic_messages(uaid, limit).await?;
        if result.messages.is_empty() {
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
        let result = self
            .primary
            .fetch_timestamp_messages(uaid, timestamp, limit)
            .await?;
        if result.messages.is_empty() {
            return self
                .secondary
                .fetch_timestamp_messages(uaid, timestamp, limit)
                .await;
        }
        return Ok(result);
    }

    async fn save_messages(&self, uaid: &Uuid, messages: Vec<Notification>) -> DbResult<()> {
        if self.write_to_secondary {
            let _ = self.secondary.save_messages(uaid, messages.clone()).await?;
        }
        self.primary.save_messages(uaid, messages).await
    }

    async fn increment_storage(&self, uaid: &Uuid, timestamp: u64) -> DbResult<()> {
        if self.write_to_secondary {
            let _ = self.secondary.increment_storage(uaid, timestamp).await?;
        }
        self.primary.increment_storage(uaid, timestamp).await
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
}

#[cfg(all(test, feature = "bigtable", feature = "dynamodb"))]
mod test {
    use super::*;
    use cadence::{NopMetricSink, StatsdClient};
    use serde_json::json;

    /// This test checks the dual parser, but also serves as a bit of
    /// documentation for how the db_settings argument should be structured
    #[test]
    fn test_args() -> DbResult<()> {
        let arg_str = json!({
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
            "write_to_secondary": true
        })
        .to_string();

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
}
