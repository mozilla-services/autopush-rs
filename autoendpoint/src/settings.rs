//! Application settings

use config::{Config, ConfigError, Environment, File};
use fernet::{Fernet, MultiFernet};
use serde::Deserialize;
use url::Url;

const DEFAULT_PORT: u16 = 8000;
const ENV_PREFIX: &str = "autoend";

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub debug: bool,
    pub port: u16,
    pub host: String,
    pub database_url: String,
    pub database_pool_max_size: Option<u32>,
    #[cfg(any(test, feature = "db_test"))]
    pub database_use_test_transactions: bool,

    pub router_table_name: String,
    pub message_table_name: String,

    pub max_data_bytes: usize,
    pub crypto_keys: String,
    pub human_logs: bool,

    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub statsd_label: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            debug: false,
            port: DEFAULT_PORT,
            host: "127.0.0.1".to_string(),
            database_url: "mysql://root@127.0.0.1/autopush".to_string(),
            database_pool_max_size: None,
            #[cfg(any(test, feature = "db_test"))]
            database_use_test_transactions: false,
            router_table_name: "router".to_string(),
            message_table_name: "message".to_string(),
            max_data_bytes: 4096,
            crypto_keys: format!("[{}]", Fernet::generate_key()),
            human_logs: false,
            statsd_host: None,
            statsd_port: 8125,
            statsd_label: "autoendpoint".to_string(),
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

    /// A simple banner for display of certain settings at startup
    pub fn banner(&self) -> String {
        let db = Url::parse(&self.database_url)
            .map(|url| url.scheme().to_owned())
            .unwrap_or_else(|_| "<invalid db>".to_owned());
        format!("http://{}:{} ({})", self.host, self.port, db)
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
