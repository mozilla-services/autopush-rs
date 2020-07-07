//! Application settings

use crate::server::FcmSettings;
use config::{Config, ConfigError, Environment, File};
use fernet::{Fernet, MultiFernet};
use serde::Deserialize;
use url::Url;

const DEFAULT_PORT: u16 = 8000;
const ENV_PREFIX: &str = "autoend";

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub debug: bool,
    pub port: u16,
    pub host: String,
    pub endpoint_url: Url,

    pub router_table_name: String,
    pub message_table_name: String,

    pub max_data_bytes: usize,
    pub crypto_keys: String,
    pub human_logs: bool,

    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub statsd_label: String,

    pub fcm: FcmSettings,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            debug: false,
            port: DEFAULT_PORT,
            host: "127.0.0.1".to_string(),
            endpoint_url: Url::parse("http://127.0.0.1:8000/").unwrap(),
            router_table_name: "router".to_string(),
            message_table_name: "message".to_string(),
            max_data_bytes: 4096,
            crypto_keys: format!("[{}]", Fernet::generate_key()),
            human_logs: false,
            statsd_host: None,
            statsd_port: 8125,
            statsd_label: "autoendpoint".to_string(),
            fcm: FcmSettings::default(),
        }
    }
}

impl Settings {
    /// Load the settings from the config file if supplied, then the environment.
    pub fn with_env_and_config_file(filename: &Option<String>) -> Result<Self, ConfigError> {
        let mut config = Config::new();

        // Merge the config file if supplied
        if let Some(config_filename) = filename {
            config.merge(File::with_name(config_filename))?;
        }

        // Merge the environment overrides
        config.merge(Environment::with_prefix(ENV_PREFIX))?;

        config.try_into::<Self>().or_else(|error| match error {
            // Configuration errors are not very sysop friendly, Try to make them
            // a bit more 3AM useful.
            ConfigError::Message(error_msg) => {
                println!("Bad configuration: {:?}", &error_msg);
                println!("Please set in config file or use environment variable.");
                println!(
                    "For example to set `database_url` use env var `{}_DATABASE_URL`\n",
                    ENV_PREFIX.to_uppercase()
                );
                error!("Configuration error: Value undefined {:?}", &error_msg);
                Err(ConfigError::NotFound(error_msg))
            }
            _ => {
                error!("Configuration error: Other: {:?}", &error);
                Err(error)
            }
        })
    }

    /// Initialize the fernet encryption instance
    pub fn make_fernet(&self) -> MultiFernet {
        if !(self.crypto_keys.starts_with('[') && self.crypto_keys.ends_with(']')) {
            panic!("Invalid AUTOEND_CRYPTO_KEY");
        }

        let crypto_keys = &self.crypto_keys[1..self.crypto_keys.len() - 1];
        let fernets = crypto_keys
            .split(',')
            .map(|key| Fernet::new(key).expect("Invalid AUTOEND_CRYPTO_KEY"))
            .collect();
        MultiFernet::new(fernets)
    }
}
