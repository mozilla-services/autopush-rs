use std::net::ToSocketAddrs;

use config::{Config, ConfigError, Environment, File};
use fernet::Fernet;
use mozsvc_common::get_hostname;

lazy_static! {
    static ref HOSTNAME: String = get_hostname().unwrap();
    static ref RESOLVED_HOSTNAME: String = get_resolved_hostname();
}

fn get_resolved_hostname() -> String {
    let hostname = get_hostname().expect("Can't get hostname");
    hostname
        .to_socket_addrs()
        .expect("Failed to resolve hostnames")
        .last()
        .expect("No hostnames found")
        .to_string()
}

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub debug: bool,
    pub port: u16,
    pub hostname: Option<String>,
    pub resolve_hostname: bool,
    pub router_port: u16,
    pub router_hostname: Option<String>,
    pub router_tablename: String,
    pub message_tablename: String,
    pub router_ssl_key: Option<String>,
    pub router_ssl_cert: Option<String>,
    pub router_ssl_dh_param: Option<String>,
    pub auto_ping_interval: f64,
    pub auto_ping_timeout: f64,
    pub max_connections: u32,
    pub close_handshake_timeout: u32,
    pub endpoint_scheme: String,
    pub endpoint_hostname: Option<String>,
    pub endpoint_port: u16,
    pub crypto_key: String,
    pub statsd_host: String,
    pub statsd_port: u16,
    pub aws_ddb_endpoint: Option<String>,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: u32,
    pub human_logs: bool,
}

impl Settings {
    /// Load the settings from the config files in order first then the environment.
    pub fn with_env_and_config_files(filenames: &[String]) -> Result<Self, ConfigError> {
        let mut s = Config::default();
        // Set our defaults, this can be fixed up drastically later after:
        // https://github.com/mehcode/config-rs/issues/60
        s.set_default("debug", false)?;
        s.set_default("port", 8080)?;
        s.set_default("resolve_hostname", false)?;
        s.set_default("router_port", 8081)?;
        s.set_default("router_tablename", "router")?;
        s.set_default("message_tablename", "message")?;
        s.set_default("auto_ping_interval", 300)?;
        s.set_default("auto_ping_timeout", 4)?;
        s.set_default("max_connections", 0)?;
        s.set_default("close_handshake_timeout", 0)?;
        s.set_default("endpoint_scheme", "http")?;
        s.set_default("endpoint_port", 8082)?;
        s.set_default("crypto_key", Fernet::generate_key())?;
        s.set_default("statsd_host", "localhost")?;
        s.set_default("statsd_port", 8125)?;
        s.set_default("megaphone_poll_interval", 30)?;
        s.set_default("human_logs", false)?;

        // Merge the configs from the files
        for filename in filenames {
            s.merge(File::with_name(filename))?;
        }

        // Merge the environment overrides
        s.merge(Environment::with_prefix("autopush"))?;
        s.try_into()
    }

    pub fn router_url(&self) -> String {
        let router_scheme = if self.router_ssl_key.is_none() {
            "http"
        } else {
            "https"
        };
        let hostname = self.host_name();
        format!(
            "{}://{}:{}",
            router_scheme,
            self.router_hostname.as_ref().unwrap_or(&hostname),
            self.router_port
        )
    }

    pub fn endpoint_url(&self) -> String {
        format!(
            "{}://{}:{}",
            self.endpoint_scheme,
            self.endpoint_hostname
                .as_ref()
                .expect("Endpoint hostname must be supplied"),
            self.endpoint_port
        )
    }

    fn host_name(&self) -> String {
        if let Some(ref hostname) = self.hostname {
            if self.resolve_hostname {
                return hostname
                    .to_socket_addrs()
                    .expect("Failed to resolve hostnames")
                    .last()
                    .expect("No hostnames found")
                    .to_string();
            } else {
                return hostname.clone();
            }
        }
        if self.resolve_hostname {
            RESOLVED_HOSTNAME.clone()
        } else {
            HOSTNAME.clone()
        }
    }
}
