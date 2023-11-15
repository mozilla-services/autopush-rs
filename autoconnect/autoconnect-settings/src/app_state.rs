use std::{sync::Arc, time::Duration};

#[cfg(feature = "bigtable")]
use autopush_common::db::bigtable::BigTableClientImpl;
use cadence::StatsdClient;
use fernet::{Fernet, MultiFernet};
use tokio::sync::RwLock;

use autoconnect_common::{
    broadcast::BroadcastChangeTracker, megaphone::init_and_spawn_megaphone_updater,
    registry::ClientRegistry,
};
use autopush_common::db::{client::DbClient, dynamodb::DdbClientImpl, DbSettings, StorageType};
use autopush_common::errors::{ApcErrorKind, Result};

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
}

impl AppState {
    pub fn from_settings(settings: Settings) -> Result<Self> {
        let crypto_key = &settings.crypto_key;
        if !(crypto_key.starts_with('[') && crypto_key.ends_with(']')) {
            return Err(
                ApcErrorKind::ConfigError(config::ConfigError::Message(format!(
                    "Invalid {}_CRYPTO_KEY",
                    ENV_PREFIX
                )))
                .into(),
            );
        }
        let crypto_key = &crypto_key[1..crypto_key.len() - 1];
        debug!("Fernet keys: {:?}", &crypto_key);
        let fernets: Vec<Fernet> = crypto_key
            .split(',')
            .map(|s| s.trim().to_string())
            .map(|key| {
                Fernet::new(&key).unwrap_or_else(|| panic!("Invalid {}_CRYPTO_KEY", ENV_PREFIX))
            })
            .collect();
        let fernet = MultiFernet::new(fernets);
        let metrics = autopush_common::metrics::builder(
            &settings.statsd_label,
            &settings.statsd_host,
            settings.statsd_port,
        )?
        // Temporary tag to distinguish from the legacy autopush(connect)
        .with_tag("autoconnect", "true")
        .build();
        let metrics = Arc::new(metrics);

        let db_settings = DbSettings {
            dsn: settings.db_dsn.clone(),
            db_settings: settings.db_settings.clone(),
        };
        let storage_type = StorageType::from_dsn(&db_settings.dsn);
        let db: Box<dyn DbClient> = match storage_type {
            StorageType::DynamoDb => Box::new(DdbClientImpl::new(metrics.clone(), &db_settings)?),
            #[cfg(feature = "bigtable")]
            StorageType::BigTable => {
                Box::new(BigTableClientImpl::new(metrics.clone(), &db_settings)?)
            }
            _ => panic!(
                "Invalid Storage type {:?}. Check {}__DB_DSN.",
                storage_type,
                ENV_PREFIX.to_uppercase()
            ),
        };
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
        })
    }

    /// Initialize the `BroadcastChangeTracker`
    ///
    /// Via `autoconnect_common::megaphone::init_and_spawn_megaphone_updater`
    pub async fn init_and_spawn_megaphone_updater(&self) -> Result<()> {
        let Some(ref url) = self.settings.megaphone_api_url else {
            return Ok(());
        };
        let Some(ref token) = self.settings.megaphone_api_token else {
            return Err(
                ApcErrorKind::ConfigError(config::ConfigError::Message(format!(
                    "{ENV_PREFIX}__MEGAPHONE_API_URL requires {ENV_PREFIX}__MEGAPHONE_API_TOKEN"
                )))
                .into(),
            );
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
        .map_err(|e| ApcErrorKind::GeneralError(format!("{}", e)))?;
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
