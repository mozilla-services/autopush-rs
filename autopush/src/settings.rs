use std::io;
use std::net::ToSocketAddrs;

use config::{Config, ConfigError, Environment, File};
use fernet::Fernet;
use lazy_static::lazy_static;
use serde_derive::Deserialize;

lazy_static! {
    static ref HOSTNAME: String = mozsvc_common::get_hostname().expect("Couldn't get_hostname").into_string().unwrap_or_else(
        |e| panic!("Could not decode hostname {:?}", e)
    );
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

#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub port: u16,
    pub hostname: Option<String>,
    pub resolve_hostname: bool,
    pub router_port: u16,
    pub router_hostname: Option<String>,
    pub router_tablename: String,
    pub message_tablename: String,
    pub meta_tablename: Option<String>,
    pub router_ssl_key: Option<String>,
    pub router_ssl_cert: Option<String>,
    pub router_ssl_dh_param: Option<String>,
    pub auto_ping_interval: f64,
    pub auto_ping_timeout: f64,
    pub max_connections: u32,
    pub close_handshake_timeout: u32,
    pub endpoint_scheme: String,
    pub endpoint_hostname: String,
    pub endpoint_port: u16,
    pub crypto_keys: String,
    pub statsd_host: String,
    pub statsd_port: u16,
    pub aws_ddb_endpoint: Option<String>,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: u32,
    pub human_logs: bool,
    pub msg_limit: u32,
    pub db_dsn: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            port: 8080,
            hostname: None,
            resolve_hostname: false,
            router_port: 8081,
            router_hostname: None,
            router_tablename: "router".to_owned(),
            message_tablename: "message".to_owned(),
            meta_tablename: None,
            router_ssl_key: None,
            router_ssl_cert: None,
            router_ssl_dh_param: None,
            auto_ping_interval: 300.0,
            auto_ping_timeout: 4.0,
            max_connections: 0,
            close_handshake_timeout: 0,
            endpoint_scheme: "http".to_owned(),
            endpoint_hostname: "localhost".to_owned(),
            endpoint_port: 8082,
            crypto_keys: format!("[{}]", Fernet::generate_key()),
            statsd_host: "localhost".to_owned(),
            statsd_port: 8125,
            aws_ddb_endpoint: None,
            megaphone_api_url: None,
            megaphone_api_token: None,
            megaphone_poll_interval: 30,
            human_logs: false,
            msg_limit: 100,
            db_dsn: None,
        }
    }
}

impl Settings {
    /// Load the settings from the config files in order first then the environment.
    pub fn with_env_and_config_files(filenames: &[String]) -> Result<Self, ConfigError> {
        let mut s = Config::builder();
        // Set our defaults, this can be fixed up drastically later after:
        // https://github.com/mehcode/config-rs/issues/60

        // Merge the configs from the files
        for filename in filenames {
            s.add_source(File::with_name(filename));
        }

        // Merge the environment overrides
        s.add_source(Environment::with_prefix("autopush").separator("__"));
        s.build().try_into()
    }

    pub fn router_url(&self) -> String {
        let router_scheme = if self.router_ssl_key.is_none() {
            "http"
        } else {
            "https"
        };
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
}

#[cfg(test)]
mod tests {
    use super::*;

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

        settings.router_port = 443;
        settings.router_ssl_key = Some("key".to_string());
        let url = settings.router_url();
        assert_eq!("https://testname", url);

        settings.router_port = 8080;
        let url = settings.router_url();
        assert_eq!("https://testname:8080", url);
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
}
