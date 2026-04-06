use std::{sync::Arc, time::Duration};

#[cfg(feature = "bigtable")]
use autopush_common::db::bigtable::BigTableClientImpl;
#[cfg(feature = "postgres")]
use autopush_common::db::postgres::PgClientImpl;
#[cfg(feature = "redis")]
use autopush_common::db::redis::RedisClientImpl;
use cadence::StatsdClient;
use config::ConfigError;
use fernet::{Fernet, MultiFernet};
use tokio::sync::RwLock;

use autoconnect_common::{
    broadcast::BroadcastChangeTracker, megaphone::init_and_spawn_megaphone_updater,
    registry::ClientRegistry,
};
use autopush_common::db::{DbSettings, StorageType, client::DbClient};
#[cfg(feature = "reliable_report")]
use autopush_common::reliability::PushReliability;

use crate::{ENV_PREFIX, Settings};

#[derive(Clone)]
pub struct AppState {
    /// Handle to the data storage object
    pub db: Box<dyn DbClient>,
    pub metrics: Arc<StatsdClient>,
    pub http: reqwest::Client,

    /// Encryption object for the endpoint URL
    pub fernet: MultiFernet,
    /// The connected WebSocket clients
    pub clients: Arc<ClientRegistry>,
    /// The Megaphone Broadcast change tracker
    pub broadcaster: Arc<RwLock<BroadcastChangeTracker>>,

    pub settings: Settings,
    pub router_url: String,
    pub endpoint_url: String,

    #[cfg(feature = "reliable_report")]
    pub reliability: Arc<PushReliability>,
}

impl AppState {
    pub fn from_settings(settings: Settings) -> Result<Self, ConfigError> {
        if settings.crypto_key.is_none() && settings.crypto_keys.is_none() {
            return Err(ConfigError::Message(format!(
                "Missing required configuration: {ENV_PREFIX}__CRYPTO_KEY or {ENV_PREFIX}__CRYPTO_KEYS"
            )));
        };
        let crypto_keys = &settings
            .crypto_keys
            .clone()
            .unwrap_or(settings.crypto_key.clone().unwrap());
        if !(crypto_keys.starts_with('[') && crypto_keys.ends_with(']')) {
            return Err(ConfigError::Message(format!(
                "Invalid {ENV_PREFIX}_CRYPTO_KEY"
            )));
        }
        let fernet_keys = &crypto_keys[1..crypto_keys.len() - 1];
        debug!("🔐 Fernet keys: {:?}", &fernet_keys);
        let fernets: Vec<Fernet> = fernet_keys
            .split(',')
            .map(|s| s.trim().to_string())
            .map(|key| {
                Fernet::new(&key).unwrap_or_else(|| panic!("Invalid {ENV_PREFIX}_CRYPTO_KEY"))
            })
            .collect();
        let fernet = MultiFernet::new(fernets);
        let metrics = autopush_common::metrics::builder(
            &settings.statsd_label,
            &settings.statsd_host,
            settings.statsd_port,
        )
        .map_err(|e| ConfigError::Message(e.to_string()))?
        // Temporary tag to distinguish from the legacy autopush(connect)
        .with_tag("autoconnect", "true")
        .build();
        let metrics = Arc::new(metrics);

        let db_settings = DbSettings {
            dsn: settings.db_dsn.clone(),
            db_settings: settings.db_settings.clone(),
        };
        let storage_type = StorageType::from_dsn(&db_settings.dsn);

        #[allow(unused)]
        let db: Box<dyn DbClient> = match storage_type {
            #[cfg(feature = "bigtable")]
            StorageType::BigTable => {
                let client = BigTableClientImpl::new(metrics.clone(), &db_settings)
                    .map_err(|e| ConfigError::Message(e.to_string()))?;
                client.spawn_sweeper(Duration::from_secs(30));
                Box::new(client)
            }
            #[cfg(feature = "postgres")]
            StorageType::Postgres => {
                let client = PgClientImpl::new(metrics.clone(), &db_settings)
                    .map_err(|e| ConfigError::Message(e.to_string()))?;
                Box::new(client)
            }
            #[cfg(feature = "redis")]
            StorageType::Redis => Box::new(
                RedisClientImpl::new(metrics.clone(), &db_settings)
                    .map_err(|e| ConfigError::Message(e.to_string()))?,
            ),
            _ => panic!(
                "Invalid Storage type {:?}. Check {}__DB_DSN.",
                storage_type,
                ENV_PREFIX.to_uppercase()
            ),
        };

        #[cfg(feature = "reliable_report")]
        let reliability = Arc::new(
            PushReliability::new(
                &settings.reliability_dsn,
                db.clone(),
                &metrics,
                settings.reliability_retry_count,
            )
            .map_err(|e| {
                ConfigError::Message(format!("Could not start Reliability connection: {e:?}"))
            })?,
        );
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .pool_max_idle_per_host(settings.pool_max_idle_per_host)
            .pool_idle_timeout(Duration::from_secs(settings.pool_idle_timeout_secs))
            .build()
            .unwrap_or_else(|e| panic!("Error while building reqwest::Client: {e}"));
        let broadcaster = Arc::new(RwLock::new(BroadcastChangeTracker::new(Vec::new())));

        let router_url = settings.router_url();
        let endpoint_url = settings.endpoint_url();

        Ok(Self {
            db,
            metrics,
            http,
            fernet,
            clients: Arc::new(ClientRegistry::with_channel_capacity(
                settings.client_channel_capacity,
            )),
            broadcaster,
            settings,
            router_url,
            endpoint_url,
            #[cfg(feature = "reliable_report")]
            reliability,
        })
    }

    /// Initialize the `BroadcastChangeTracker`
    ///
    /// Via `autoconnect_common::megaphone::init_and_spawn_megaphone_updater`
    pub async fn init_and_spawn_megaphone_updater(&self) -> Result<(), ConfigError> {
        let Some(ref url) = self.settings.megaphone_api_url else {
            return Ok(());
        };
        let Some(ref token) = self.settings.megaphone_api_token else {
            return Err(ConfigError::Message(format!(
                "{ENV_PREFIX}__MEGAPHONE_API_URL requires {ENV_PREFIX}__MEGAPHONE_API_TOKEN"
            )));
        };
        init_and_spawn_megaphone_updater(
            &self.broadcaster,
            &self.http,
            &self.metrics,
            url,
            token,
            self.settings.megaphone_poll_interval,
        )
        .await
        .map_err(|e| ConfigError::Message(e.to_string()))?;
        Ok(())
    }
}

/// For tests
#[cfg(debug_assertions)]
impl Default for AppState {
    fn default() -> Self {
        Self::from_settings(Settings::test_settings()).unwrap()
    }
}
