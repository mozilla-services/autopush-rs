use std::io;
use std::net::ToSocketAddrs;

use config::{Config, ConfigError, Environment, File};
use fernet::Fernet;
use lazy_static::lazy_static;
use serde_derive::Deserialize;

const ENV_PREFIX: &str = "autoconnect";

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

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct Settings {
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
    pub endpoint_hostname: String,
    pub endpoint_port: u16,
    pub crypto_key: String,
    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub statsd_label: String,
    pub db_dsn: Option<String>,
    pub db_settings: String,
    // pub aws_ddb_endpoint: Option<String>,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: u32,
    pub human_logs: bool,
    pub msg_limit: u32,
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
            crypto_key: format!("[{}]", Fernet::generate_key()),
            statsd_host: Some("localhost".to_owned()),
            statsd_label: ENV_PREFIX.to_owned(),
            statsd_port: 8125,
            db_dsn: None,
            db_settings: "".to_owned(),
            megaphone_api_url: None,
            megaphone_api_token: None,
            megaphone_poll_interval: 30,
            human_logs: false,
            msg_limit: 100,
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
        s = s.add_source(Environment::with_prefix(ENV_PREFIX).separator("__"));
        // s = s.add_source(Environment::with_prefix(ENV_PREFIX));

        let built = s.build()?;
        built.try_deserialize::<Settings>()
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
