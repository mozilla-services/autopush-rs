//! Application settings

use crate::routers::adm::settings::AdmSettings;
use crate::routers::apns::settings::ApnsSettings;
use crate::routers::fcm::settings::FcmSettings;
use config::{Config, ConfigError, Environment, File};
use fernet::{Fernet, MultiFernet};
use serde::Deserialize;
use url::Url;

pub const ENV_PREFIX: &str = "autoend";

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub scheme: String,
    pub host: String,
    pub port: u16,
    pub endpoint_url: String,

    pub use_ddb: bool,
    pub pg_dsn: Option<String>,
    pub router_table_name: String,
    pub message_table_name: String,
    pub meta_table_name: Option<String>,

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
            scheme: "http".to_string(),
            host: "127.0.0.1".to_string(),
            endpoint_url: "".to_string(),
            port: 8000,
            use_ddb: true,
            pg_dsn: None,
            router_table_name: "router".to_string(),
            message_table_name: "message".to_string(),
            meta_table_name: None,
            max_data_bytes: 4096,
            crypto_keys: format!("[{}]", Fernet::generate_key()),
            auth_keys: r#"["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB="]"#.to_string(),
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
        let mut s = Config::default();

        // Merge the config file if supplied
        if let Some(config_filename) = filename {
            s.merge(File::with_name(config_filename))?;
        }

        // Merge the environment overrides
        // Note: Specify the separator here so that the shell can properly pass args
        // down to the sub structures. Also, with config 0.12 the `separator` impacts
        // which vars are read. (e.g. `separator('__') ignores "AUTOEND_FOO__BAR" since
        // the "key" would be determined as "AUTOEND_FOO"). Config::Environment
        // overloads separator and group_separator with no way to differentiate. The
        // better solution is to NOT specify `separator()` and use the default
        // `_` group separator.
        s.merge(Environment::with_prefix(ENV_PREFIX).separator("__"))?;

        s.try_into::<Self>().map_err(|error| match error {
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
        let keys = &self.crypto_keys.replace('"', "").replace(' ', "");
        let fernets = Self::read_list_from_str(keys, "Invalid AUTOEND_CRYPTO_KEYS")
            .map(|key| Fernet::new(key).expect("Invalid AUTOEND_CRYPTO_KEYS"))
            .collect();
        MultiFernet::new(fernets)
    }

    /// Get the list of auth hash keys
    pub fn auth_keys(&self) -> Vec<String> {
        let keys = &self.auth_keys.replace('"', "").replace(' ', "");
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
}
