//! Application settings

use crate::routers::adm::settings::AdmSettings;
use crate::routers::apns::settings::ApnsSettings;
use crate::routers::fcm::settings::FcmSettings;
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
    pub auth_keys: String,
    pub human_logs: bool,

    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub statsd_label: String,

    pub fcm: FcmSettings,
    pub apns: ApnsSettings,
    pub adm: AdmSettings,
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
            auth_keys: "[AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB=]".to_string(),
            human_logs: false,
            statsd_host: None,
            statsd_port: 8125,
            statsd_label: "autoendpoint".to_string(),
            fcm: FcmSettings::default(),
            apns: ApnsSettings::default(),
            adm: AdmSettings::default(),
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

        config.try_into::<Self>().map_err(|error| match error {
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
                ConfigError::NotFound(error_msg)
            }
            _ => {
                error!("Configuration error: Other: {:?}", &error);
                error
            }
        })
    }

    /// Convert a string like `[item1,item2]` into a iterator over `item1` and `item2`.
    /// Panics with a custom message if the string is not in the expected form.
    fn read_list_from_str<'list>(
        list_str: &'list str,
        panic_msg: &'static str,
    ) -> impl Iterator<Item = &'list str> {
        if !(list_str.starts_with('[') && list_str.ends_with(']')) {
            panic!(panic_msg);
        }

        let items = &list_str[1..list_str.len() - 1];
        items.split(',')
    }

    /// Initialize the fernet encryption instance
    pub fn make_fernet(&self) -> MultiFernet {
        let fernets = Self::read_list_from_str(&self.crypto_keys, "Invalid AUTOEND_CRYPTO_KEYS")
            .map(|key| Fernet::new(key).expect("Invalid AUTOEND_CRYPTO_KEYS"))
            .collect();
        MultiFernet::new(fernets)
    }

    /// Get the list of auth hash keys
    pub fn auth_keys(&self) -> Vec<&str> {
        Self::read_list_from_str(&self.auth_keys, "Invalid AUTOEND_AUTH_KEYS").collect()
    }
}
