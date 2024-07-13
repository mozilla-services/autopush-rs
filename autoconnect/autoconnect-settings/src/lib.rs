mod app_state;

extern crate slog;
#[macro_use]
extern crate slog_scope;
extern crate serde_derive;

use std::{io, net::ToSocketAddrs, time::Duration};

use config::{Config, ConfigError, Environment, File};
use fernet::Fernet;
use lazy_static::lazy_static;
use serde::{Deserialize, Deserializer};
use serde_json::json;

use autopush_common::util::deserialize_u32_to_duration;

pub use app_state::AppState;

pub const ENV_PREFIX: &str = "autoconnect";

lazy_static! {
    static ref HOSTNAME: String = mozsvc_common::get_hostname()
        .expect("Couldn't get_hostname")
        .into_string()
        .expect("Couldn't convert get_hostname");
    static ref RESOLVED_HOSTNAME: String = resolve_ip(&HOSTNAME)
        .unwrap_or_else(|_| panic!("Failed to resolve hostname: {}", *HOSTNAME));
}

/// Resolve a hostname to its IP if possible
fn resolve_ip(hostname: &str) -> io::Result<String> {
    Ok((hostname, 0)
        .to_socket_addrs()?
        .next()
        .map_or_else(|| hostname.to_owned(), |addr| addr.ip().to_string()))
}

/// Indicate whether the port should be included for the given scheme
fn include_port(scheme: &str, port: u16) -> bool {
    !((scheme == "http" && port == 80) || (scheme == "https" && port == 443))
}

/// The Applications settings, read from CLI, Environment or settings file, for the
/// autoconnect application. These are later converted to
/// [autoconnect::autoconnect-settings::AppState].
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The application port to listen on
    pub port: u16,
    /// The DNS specified name of the application host to used for internal routing
    pub hostname: Option<String>,
    /// The override hostname to use for internal routing (NOTE: requires `hostname` to be set)
    pub resolve_hostname: bool,
    /// The internal webpush routing port
    pub router_port: u16,
    /// The DNS name to use for internal routing
    pub router_hostname: Option<String>,
    /// The server based ping interval (also used for Broadcast sends)
    #[serde(deserialize_with = "deserialize_f64_to_duration")]
    pub auto_ping_interval: Duration,
    /// How long to wait for a response Pong before being timed out and connection drop
    #[serde(deserialize_with = "deserialize_f64_to_duration")]
    pub auto_ping_timeout: Duration,
    /// How long to wait for the initial connection handshake.
    #[serde(deserialize_with = "deserialize_u32_to_duration")]
    pub open_handshake_timeout: Duration,
    /// How long to wait while closing a connection for the response handshake.
    #[serde(deserialize_with = "deserialize_u32_to_duration")]
    pub close_handshake_timeout: Duration,
    /// The URL scheme (http/https) for the endpoint URL
    pub endpoint_scheme: String,
    /// The host url for the endpoint URL (differs from `hostname` and `resolve_hostname`)
    pub endpoint_hostname: String,
    /// The optional port override for the endpoint URL
    pub endpoint_port: u16,
    /// The seed key to use for endpoint encryption
    pub crypto_key: String,
    /// The host name to send recorded metrics
    pub statsd_host: Option<String>,
    /// The port number to send recorded metrics
    pub statsd_port: u16,
    /// The root label to apply to metrics.
    pub statsd_label: String,
    /// The DSN to connect to the storage engine (Used to select between storage systems)
    pub db_dsn: Option<String>,
    /// JSON set of specific database settings (See data storage engines)
    pub db_settings: String,
    /// Server endpoint to pull Broadcast ID change values (Sent in Pings)
    pub megaphone_api_url: Option<String>,
    /// Broadcast token for authentication
    pub megaphone_api_token: Option<String>,
    /// How often to poll the server for new data
    #[serde(deserialize_with = "deserialize_u32_to_duration")]
    pub megaphone_poll_interval: Duration,
    /// Use human readable (simplified, non-JSON)
    pub human_logs: bool,
    /// Maximum allowed number of backlogged messages. Exceeding this number will
    /// trigger a user reset because the user may have been offline way too long.
    pub msg_limit: u32,
    /// Sets the maximum number of concurrent connections per actix-web worker.
    ///
    /// All socket listeners will stop accepting connections when this limit is
    /// reached for each worker.
    pub actix_max_connections: Option<usize>,
    /// Sets number of actix-web workers to start (per bind address).
    ///
    /// By default, the number of available physical CPUs is used as the worker count.
    pub actix_workers: Option<usize>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 8080,
            hostname: None,
            resolve_hostname: false,
            router_port: 8081,
            router_hostname: None,
            auto_ping_interval: Duration::from_secs(300),
            auto_ping_timeout: Duration::from_secs(4),
            open_handshake_timeout: Duration::from_secs(5),
            close_handshake_timeout: Duration::from_secs(0),
            endpoint_scheme: "http".to_owned(),
            endpoint_hostname: "localhost".to_owned(),
            endpoint_port: 8082,
            crypto_key: format!("[{}]", Fernet::generate_key()),
            statsd_host: Some("localhost".to_owned()),
            // Matches the legacy value
            statsd_label: "autopush".to_owned(),
            statsd_port: 8125,
            db_dsn: None,
            db_settings: "".to_owned(),
            megaphone_api_url: None,
            megaphone_api_token: None,
            megaphone_poll_interval: Duration::from_secs(30),
            human_logs: false,
            msg_limit: 100,
            actix_max_connections: None,
            actix_workers: None,
        }
    }
}

impl Settings {
    /// Load the settings from the config files in order first then the environment.
    pub fn with_env_and_config_files(filenames: &[String]) -> Result<Self, ConfigError> {
        let mut s = Config::builder();

        // Merge the configs from the files
        for filename in filenames {
            s = s.add_source(File::with_name(filename));
        }

        // Merge the environment overrides
        s = s.add_source(Environment::with_prefix(&ENV_PREFIX.to_uppercase()).separator("__"));

        let built = s.build()?;
        let s = built.try_deserialize::<Settings>()?;
        s.validate()?;
        Ok(s)
    }

    pub fn router_url(&self) -> String {
        let router_scheme = "http";
        let url = format!(
            "{}://{}",
            router_scheme,
            self.router_hostname
                .as_ref()
                .map_or_else(|| self.get_hostname(), String::clone),
        );
        if include_port(router_scheme, self.router_port) {
            format!("{}:{}", url, self.router_port)
        } else {
            url
        }
    }

    pub fn endpoint_url(&self) -> String {
        let url = format!("{}://{}", self.endpoint_scheme, self.endpoint_hostname,);
        if include_port(&self.endpoint_scheme, self.endpoint_port) {
            format!("{}:{}", url, self.endpoint_port)
        } else {
            url
        }
    }

    fn get_hostname(&self) -> String {
        if let Some(ref hostname) = self.hostname {
            if self.resolve_hostname {
                resolve_ip(hostname)
                    .unwrap_or_else(|_| panic!("Failed to resolve provided hostname: {}", hostname))
            } else {
                hostname.clone()
            }
        } else if self.resolve_hostname {
            RESOLVED_HOSTNAME.clone()
        } else {
            HOSTNAME.clone()
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let non_zero = |val: Duration, name| {
            if val.is_zero() {
                return Err(ConfigError::Message(format!(
                    "Invalid {}_{}: cannot be 0",
                    ENV_PREFIX, name
                )));
            }
            Ok(())
        };
        non_zero(self.megaphone_poll_interval, "MEGAPHONE_POLL_INTERVAL")?;
        non_zero(self.auto_ping_interval, "AUTO_PING_INTERVAL")?;
        non_zero(self.auto_ping_timeout, "AUTO_PING_TIMEOUT")?;
        Ok(())
    }

    pub fn test_settings() -> Self {
        let db_dsn = Some("grpc://localhost:8086".to_string());
        // BigTable DB_SETTINGS.
        let db_settings = json!({
            "table_name":"projects/test/instances/test/tables/autopush",
            "message_family":"message",
            "router_family":"router",
            "message_topic_family":"message_topic",
        })
        .to_string();
        Self {
            db_dsn,
            db_settings,
            ..Default::default()
        }
    }
}

fn deserialize_f64_to_duration<'de, D>(deserializer: D) -> Result<Duration, D::Error>
where
    D: Deserializer<'de>,
{
    let seconds: f64 = Deserialize::deserialize(deserializer)?;
    Ok(Duration::new(
        seconds as u64,
        (seconds.fract() * 1_000_000_000.0) as u32,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use slog_scope::trace;

    #[test]
    fn test_router_url() {
        let mut settings = Settings {
            router_hostname: Some("testname".to_string()),
            router_port: 80,
            ..Default::default()
        };
        let url = settings.router_url();
        assert_eq!("http://testname", url);

        settings.router_port = 8080;
        let url = settings.router_url();
        assert_eq!("http://testname:8080", url);
    }

    #[test]
    fn test_endpoint_url() {
        let mut settings = Settings {
            endpoint_hostname: "testname".to_string(),
            endpoint_port: 80,
            endpoint_scheme: "http".to_string(),
            ..Default::default()
        };
        let url = settings.endpoint_url();
        assert_eq!("http://testname", url);

        settings.endpoint_port = 8080;
        let url = settings.endpoint_url();
        assert_eq!("http://testname:8080", url);

        settings.endpoint_port = 443;
        settings.endpoint_scheme = "https".to_string();
        let url = settings.endpoint_url();
        assert_eq!("https://testname", url);

        settings.endpoint_port = 8080;
        let url = settings.endpoint_url();
        assert_eq!("https://testname:8080", url);
    }

    #[test]
    fn test_default_settings() {
        // Test that the Config works the way we expect it to.
        use std::env;
        let port = format!("{}__PORT", ENV_PREFIX).to_uppercase();
        let msg_limit = format!("{}__MSG_LIMIT", ENV_PREFIX).to_uppercase();
        let fernet = format!("{}__CRYPTO_KEY", ENV_PREFIX).to_uppercase();

        let v1 = env::var(&port);
        let v2 = env::var(&msg_limit);
        env::set_var(&port, "9123");
        env::set_var(&msg_limit, "123");
        env::set_var(&fernet, "[mqCGb8D-N7mqx6iWJov9wm70Us6kA9veeXdb8QUuzLQ=]");
        let settings = Settings::with_env_and_config_files(&Vec::new()).unwrap();
        assert_eq!(settings.endpoint_hostname, "localhost".to_owned());
        assert_eq!(&settings.port, &9123);
        assert_eq!(&settings.msg_limit, &123);
        assert_eq!(
            &settings.crypto_key,
            "[mqCGb8D-N7mqx6iWJov9wm70Us6kA9veeXdb8QUuzLQ=]"
        );
        assert_eq!(settings.open_handshake_timeout, Duration::from_secs(5));

        // reset (just in case)
        if let Ok(p) = v1 {
            trace!("Resetting {}", &port);
            env::set_var(&port, p);
        } else {
            env::remove_var(&port);
        }
        if let Ok(p) = v2 {
            trace!("Resetting {}", msg_limit);
            env::set_var(&msg_limit, p);
        } else {
            env::remove_var(&msg_limit);
        }
        env::remove_var(&fernet);
    }
}
