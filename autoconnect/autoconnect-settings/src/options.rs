use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cadence::StatsdClient;
use fernet::{Fernet, MultiFernet};

use crate::{Settings, ENV_PREFIX};
use autoconnect_registry::ClientRegistry;
use autopush_common::db::{
    bigtable::BigTableClientImpl,
    client::DbClient,
    dynamodb::DdbClientImpl,
    DbSettings,
    StorageType};
use autopush_common::{
    errors::{ApcErrorKind, Result},
    metrics::new_metrics,
};

fn ito_dur(seconds: u32) -> Option<Duration> {
    if seconds == 0 {
        None
    } else {
        Some(Duration::new(seconds.into(), 0))
    }
}

fn fto_dur(seconds: f64) -> Option<Duration> {
    if seconds == 0.0 {
        None
    } else {
        Some(Duration::new(
            seconds as u64,
            (seconds.fract() * 1_000_000_000.0) as u32,
        ))
    }
}

/// A thread safe set of options specific to the Server. These are compiled from [crate::Settings]
#[derive(Clone)]
pub struct AppState {
    pub router_port: u16,
    pub port: u16,
    /// Encryption object for the endpoint URL
    pub fernet: MultiFernet,
    pub metrics: Arc<StatsdClient>,
    /// Handle to the data storage object
    pub db_client: Box<dyn DbClient>,
    pub ssl_key: Option<PathBuf>,
    pub ssl_cert: Option<PathBuf>,
    pub ssl_dh_param: Option<PathBuf>,
    pub open_handshake_timeout: Option<Duration>,
    pub auto_ping_interval: Duration,
    pub auto_ping_timeout: Duration,
    pub max_connections: Option<u32>,
    pub close_handshake_timeout: Option<Duration>,
    pub router_url: String,
    pub endpoint_url: String,
    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: Duration,
    pub human_logs: bool,
    pub msg_limit: u32,
    pub registry: Arc<ClientRegistry>,
    pub max_pending_notification_queue: usize,
}

impl AppState {
    pub fn from_settings(settings: &Settings) -> Result<Self> {
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
        let metrics = Arc::new(new_metrics(
            settings.statsd_host.clone(),
            settings.statsd_port,
        )?);

        let router_url = settings.router_url();
        let endpoint_url = settings.endpoint_url();

        let db_settings = DbSettings {
            dsn: settings.db_dsn.clone(),
            db_settings: settings.db_settings.clone(),
        };
        let db_client:Box<dyn DbClient> = match StorageType::from_dsn(&db_settings.dsn) {
            StorageType::DynamoDb => Box::new(DdbClientImpl::new(metrics.clone(), &db_settings)?),
            StorageType::BigTable => Box::new(BigTableClientImpl::new(metrics.clone(), &db_settings)?),
            _ => panic!("Invalid Storage type. Check {}_DB_DSN.", ENV_PREFIX),
        };
        Ok(Self {
            port: settings.port,
            fernet,
            metrics,
            db_client,
            router_port: settings.router_port,
            statsd_host: settings.statsd_host.clone(),
            statsd_port: settings.statsd_port,
            router_url,
            endpoint_url,
            ssl_key: settings.router_ssl_key.clone().map(PathBuf::from),
            ssl_cert: settings.router_ssl_cert.clone().map(PathBuf::from),
            ssl_dh_param: settings.router_ssl_dh_param.clone().map(PathBuf::from),
            auto_ping_interval: fto_dur(settings.auto_ping_interval)
                .expect("auto ping interval cannot be 0"),
            auto_ping_timeout: fto_dur(settings.auto_ping_timeout)
                .expect("auto ping timeout cannot be 0"),
            close_handshake_timeout: ito_dur(settings.close_handshake_timeout),
            max_connections: if settings.max_connections == 0 {
                None
            } else {
                Some(settings.max_connections)
            },
            open_handshake_timeout: ito_dur(5),
            megaphone_api_url: settings.megaphone_api_url.clone(),
            megaphone_api_token: settings.megaphone_api_token.clone(),
            megaphone_poll_interval: ito_dur(settings.megaphone_poll_interval)
                .expect("megaphone poll interval cannot be 0"),
            human_logs: settings.human_logs,
            msg_limit: settings.msg_limit,
            registry: Arc::new(ClientRegistry::default()),
            max_pending_notification_queue: settings.max_pending_notification_queue as usize,
        })
    }
}
