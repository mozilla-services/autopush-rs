//! Application settings

use actix_http::header::HeaderMap;
use config::{Config, ConfigError, Environment, File};
use fernet::{Fernet, MultiFernet};
use serde::Deserialize;
use url::Url;

use crate::headers::vapid::VapidHeaderWithKey;
use crate::routers::apns::settings::ApnsSettings;
use crate::routers::fcm::settings::FcmSettings;
#[cfg(feature = "stub")]
use crate::routers::stub::settings::StubSettings;

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

    pub vapid_aud: Vec<String>,

    /// A stringified JSON list of VAPID public keys which should be tracked internally.
    /// This should ONLY include Mozilla generated and consumed messages (e.g. "SendToTab", etc.)
    /// These keys should be specified in stripped, b64encoded, X962 format (e.g. a single line of
    /// base64 encoded data without padding).
    /// You can use `scripts/convert_pem_to_x962.py` to easily convert EC Public keys stored in
    /// PEM format into appropriate x962 format.
    pub tracking_keys: String,
    /// Cached, parsed tracking keys.
    //TODO: convert to decoded Vec<u8>?
    pub tracking_vapid_pubs: Vec<String>,

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
    #[cfg(feature = "stub")]
    pub stub: StubSettings,
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
            vapid_aud: vec![
                "https://push.services.mozilla.org".to_string(),
                "http://127.0.0.1:9160".to_string(),
            ],
            tracking_vapid_pubs: vec![],
            // max data is a bit hard to figure out, due to encryption. Using something
            // like pywebpush, if you encode a block of 4096 bytes, you'll get a
            // 4216 byte data block. Since we're going to be receiving this, we have to
            // presume base64 encoding, so we can bump things up to 5630 bytes max.
            max_data_bytes: 5630,
            crypto_keys: format!("[{}]", Fernet::generate_key()),
            auth_keys: r#"["AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAB="]"#.to_string(),
            tracking_keys: r#"[]"#.to_string(),
            human_logs: false,
            connection_timeout_millis: 1000,
            request_timeout_millis: 3000,
            statsd_host: None,
            statsd_port: 8125,
            statsd_label: "autoendpoint".to_string(),
            fcm: FcmSettings::default(),
            apns: ApnsSettings::default(),
            #[cfg(feature = "stub")]
            stub: StubSettings::default(),
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

        let mut built: Self = config.build()?.try_deserialize::<Self>().map_err(|error| {
            match error {
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
            }
        })?;

        // cache the tracking keys we've built.
        built.tracking_vapid_pubs = built.tracking_keys();

        Ok(built)
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

    /// Get the list of tracking public keys
    pub fn tracking_keys(&self) -> Vec<String> {
        // return the cached version if present.
        if !self.tracking_vapid_pubs.is_empty() {
            return self.tracking_vapid_pubs.clone();
        };
        let keys = &self.tracking_keys.replace(['"', ' '], "");
        Self::read_list_from_str(keys, "Invalid AUTOEND_TRACKING_KEYS")
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

#[derive(Clone, Debug)]
pub struct VapidTracker(pub Vec<String>);
impl VapidTracker {
    /// Very simple string check to see if the Public Key specified in the Vapid header
    /// matches the set of trackable keys.
    pub fn is_trackable(&self, vapid: &VapidHeaderWithKey) -> bool {
        // ideally, [Settings.with_env_and_config_file()] does the work of pre-populating
        // the Settings.tracking_vapid_pubs cache, but we can't rely on that.
        self.0.contains(&vapid.public_key)
    }

    /// Extract the message Id from the headers (if present), otherwise just make one up.
    pub fn get_tracking_id(&self, headers: &HeaderMap) -> String {
        headers
            .get("X-MessageId")
            .map(|s| {
                s.to_str()
                    .unwrap_or(uuid::Uuid::new_v4().as_simple().to_string().as_str())
                    .to_string()
            })
            .unwrap_or(uuid::Uuid::new_v4().as_simple().to_string())
    }
}

#[cfg(test)]
mod tests {
    use actix_http::header::{HeaderMap, HeaderName, HeaderValue};

    use super::{Settings, VapidTracker};
    use crate::{
        error::ApiResult,
        headers::vapid::{VapidHeader, VapidHeaderWithKey},
    };

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

    #[test]
    fn test_tracking_keys() -> ApiResult<()> {
        let settings = Settings{
            tracking_keys: r#"["BLMymkOqvT6OZ1o9etCqV4jGPkvOXNz5FdBjsAR9zR5oeCV1x5CBKuSLTlHon-H_boHTzMtMoNHsAGDlDB6X7vI"]"#.to_owned(),
            ..Default::default()
        };

        let test_header = VapidHeaderWithKey {
            vapid: VapidHeader {
                scheme: "".to_owned(),
                token: "".to_owned(),
                version_data: crate::headers::vapid::VapidVersionData::Version1,
            },
            public_key: "BLMymkOqvT6OZ1o9etCqV4jGPkvOXNz5FdBjsAR9zR5oeCV1x5CBKuSLTlHon-H_boHTzMtMoNHsAGDlDB6X7vI".to_owned()
        };

        let key_set = settings.tracking_keys();
        assert!(!key_set.is_empty());

        let reliability = VapidTracker(key_set);
        assert!(reliability.is_trackable(&test_header));

        Ok(())
    }

    #[test]
    fn test_tracking_id() -> ApiResult<()> {
        let mut headers = HeaderMap::new();
        let keys = Vec::new();
        let reliability = VapidTracker(keys);

        let key = reliability.get_tracking_id(&headers);
        assert!(!key.is_empty());

        headers.insert(
            HeaderName::from_lowercase(b"x-messageid").unwrap(),
            HeaderValue::from_static("123foobar456"),
        );

        let key = reliability.get_tracking_id(&headers);
        assert_eq!(key, "123foobar456".to_owned());

        Ok(())
    }
}
