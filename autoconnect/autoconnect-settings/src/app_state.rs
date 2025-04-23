use std::{sync::Arc, time::Duration};

#[cfg(feature = "bigtable")]
use autopush_common::db::bigtable::BigTableClientImpl;
use cadence::StatsdClient;
use config::ConfigError;
use fernet::{Fernet, MultiFernet};
use tokio::sync::RwLock;

use autoconnect_common::{
    broadcast::BroadcastChangeTracker, megaphone::init_and_spawn_megaphone_updater,
    registry::ClientRegistry,
};
use autopush_common::db::{client::DbClient, DbSettings, StorageType};
#[cfg(feature = "reliable_report")]
use autopush_common::reliability::PushReliability;

use crate::{Settings, ENV_PREFIX};

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
        let crypto_key = &settings.crypto_key;
        if !(crypto_key.starts_with('[') && crypto_key.ends_with(']')) {
            return Err(ConfigError::Message(format!(
                "Invalid {ENV_PREFIX}_CRYPTO_KEY"
            )));
        }
        let crypto_key = &crypto_key[1..crypto_key.len() - 1];
        debug!("🔐 Fernet keys: {:?}", &crypto_key);
        let fernets: Vec<Fernet> = crypto_key
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
                ConfigError::Message(format!("Could not start Reliability connection: {:?}", e))
            })?,
        );
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .unwrap_or_else(|e| panic!("Error while building reqwest::Client: {}", e));
        let broadcaster = Arc::new(RwLock::new(BroadcastChangeTracker::new(Vec::new())));

        let router_url = settings.router_url();
        let endpoint_url = settings.endpoint_url();

        Ok(Self {
            db,
            metrics,
            http,
            fernet,
            clients: Arc::new(ClientRegistry::default()),
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
