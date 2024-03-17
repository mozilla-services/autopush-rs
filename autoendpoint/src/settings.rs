//! Application settings

use config::{Config, ConfigError, Environment, File};
use fernet::{Fernet, MultiFernet};
use serde::Deserialize;
use url::Url;

#[cfg(feature = "adm")]
use crate::routers::adm::settings::AdmSettings;
use crate::routers::apns::settings::ApnsSettings;
use crate::routers::fcm::settings::FcmSettings;

pub const ENV_PREFIX: &str = "autoend";

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub endpoint_url: String,

    /// The DSN to connect to the storage engine (Used to select between storage systems)
    pub db_dsn: Option<String>,
    /// JSON set of specific database settings (See data storage engines)
    pub db_settings: String,

    pub router_table_name: String,
    pub message_table_name: String,

    pub max_data_bytes: usize,
    pub crypto_keys: String,
    pub auth_keys: String,
    pub human_logs: bool,

    pub connection_timeout_millis: u64,
    pub request_timeout_millis: u64,

    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub statsd_label: String,

    pub fcm: FcmSettings,
    pub apns: ApnsSettings,
    #[cfg(feature = "adm")]
    pub adm: AdmSettings,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            scheme: "http".to_string(),
            host: "127.0.0.1".to_string(),
            endpoint_url: "".to_string(),
            port: 8000,
            db_dsn: None,
            db_settings: "".to_owned(),
            router_table_name: "router".to_string(),
            message_table_name: "message".to_string(),
            // max data is a bit hard to figure out, due to encryption. Using something
            // like pywebpush, if you encode a block of 4096 bytes, you'll get a
            // 4216 byte data block. Since we're going to be receiving this, we have to
            // presume base64 encoding, so we can bump things up to 5630 bytes max.
            max_data_bytes: 5630,
            crypto_keys: format!("[{}]", Fernet::generate_key()),
            auth_keys: r#"["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB="]"#.to_string(),
            human_logs: false,
            connection_timeout_millis: 1000,
            request_timeout_millis: 3000,
            statsd_host: None,
            statsd_port: 8125,
            statsd_label: "autoendpoint".to_string(),
            fcm: FcmSettings::default(),
            apns: ApnsSettings::default(),
            #[cfg(feature = "adm")]
            adm: AdmSettings::default(),
        }
    }
}

impl Settings {
    /// Load the settings from the config file if supplied, then the environment.
    pub fn with_env_and_config_file(filename: &Option<String>) -> Result<Self, ConfigError> {
        let mut config = Config::builder();

        // Merge the config file if supplied
        if let Some(config_filename) = filename {
            config = config.add_source(File::with_name(config_filename));
        }

        // Merge the environment overrides
        // Note: Specify the separator here so that the shell can properly pass args
        // down to the sub structures.
        config = config.add_source(Environment::with_prefix(ENV_PREFIX).separator("__"));

        let built = config.build()?;

        built
            .try_deserialize::<Self>()
            .map_err(|error| match error {
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
            panic!("{}", panic_msg);
        }

        let items = &list_str[1..list_str.len() - 1];
        items.split(',')
    }

    /// Initialize the fernet encryption instance
    pub fn make_fernet(&self) -> MultiFernet {
        let keys = &self.crypto_keys.replace(['"', ' '], "");
        let fernets = Self::read_list_from_str(keys, "Invalid AUTOEND_CRYPTO_KEYS")
            .map(|key| {
                debug!("🔐 Fernet keys: {:?}", &key);
                Fernet::new(key).expect("Invalid AUTOEND_CRYPTO_KEYS")
            })
            .collect();
        MultiFernet::new(fernets)
    }

    /// Get the list of auth hash keys
    pub fn auth_keys(&self) -> Vec<String> {
        let keys = &self.auth_keys.replace(['"', ' '], "");
        Self::read_list_from_str(keys, "Invalid AUTOEND_AUTH_KEYS")
            .map(|v| v.to_owned())
            .collect()
    }

    /// Get the URL for this endpoint server
    pub fn endpoint_url(&self) -> Url {
        let endpoint = if self.endpoint_url.is_empty() {
            format!("{}://{}:{}", self.scheme, self.host, self.port)
        } else {
            self.endpoint_url.clone()
        };
        Url::parse(&endpoint).expect("Invalid endpoint URL")
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;
    use crate::error::ApiResult;

    #[test]
    fn test_auth_keys() -> ApiResult<()> {
        let success: Vec<String> = vec![
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB=".to_owned(),
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAC=".to_owned(),
        ];
        // Try with quoted strings
        let settings = Settings{
            auth_keys: r#"["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB=", "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAC="]"#.to_owned(),
            ..Default::default()
        };
        let result = settings.auth_keys();
        assert_eq!(result, success);

        // try with unquoted, non-JSON compliant strings.
        let settings = Settings{
            auth_keys: r#"[AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB=,AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAC=]"#.to_owned(),
            ..Default::default()
        };
        let result = settings.auth_keys();
        assert_eq!(result, success);
        Ok(())
    }

    #[test]
    fn test_endpoint_url() -> ApiResult<()> {
        let example = "https://example.org/";
        let settings = Settings {
            endpoint_url: example.to_owned(),
            ..Default::default()
        };

        assert_eq!(settings.endpoint_url(), url::Url::parse(example).unwrap());
        let settings = Settings {
            ..Default::default()
        };

        assert_eq!(
            settings.endpoint_url(),
            url::Url::parse(&format!(
                "{}://{}:{}",
                settings.scheme, settings.host, settings.port
            ))
            .unwrap()
        );
        Ok(())
    }

    #[test]
    fn test_default_settings() {
        // Test that the Config works the way we expect it to.
        let port = format!("{}__PORT", super::ENV_PREFIX).to_uppercase();
        let timeout = format!("{}__FCM__TIMEOUT", super::ENV_PREFIX).to_uppercase();

        use std::env;
        let v1 = env::var(&port);
        let v2 = env::var(&timeout);
        env::set_var(&port, "9123");
        env::set_var(&timeout, "123");

        let settings = Settings::with_env_and_config_file(&None).unwrap();
        assert_eq!(&settings.port, &9123);
        assert_eq!(&settings.fcm.timeout, &123);
        assert_eq!(settings.host, "127.0.0.1".to_owned());
        // reset (just in case)
        if let Ok(p) = v1 {
            trace!("Resetting {}", &port);
            env::set_var(&port, p);
        } else {
            env::remove_var(&port);
        }
        if let Ok(p) = v2 {
            trace!("Resetting {}", &timeout);
            env::set_var(&timeout, p);
        } else {
            env::remove_var(&timeout);
        }
    }
}
