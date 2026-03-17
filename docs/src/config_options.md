# Configuration options

Configuration options can be specified either as environment variables or as values in a
configuration file. Configuration files can be specified using the Command Line Interface (CLI) argument "-c"

## Common CLI args

| **short form** | **long form** | **description** |
|---|---|---|
| -h | --help | Display command line arguments |
| -c | --config _config/file/path_ | Path to a configuration file (e.g. `config.toml`) to use instead of environment variables |

## Autoconnect

The following configuration options can be specified either in the configuration file or by
using their environment variable form.

| **option** | **env var** | **type** | **default value** | **description** |
|---|---|---|---|---|
| <span id="AUTOCONNECT__ACTIX_MAX_CONNECTIONS" />actix_max_connections | AUTOCONNECT__ACTIX_MAX_CONNECTIONS | num | _None_ | Maximum number of concurrent connections for the Actix web server |
| <span id="AUTOCONNECT__ACTIX_WORKERS" />actix_workers | AUTOCONNECT__ACTIX_WORKERS | num | _None_ | Number of worker threads for the Actix web server per bind address |
| <span id="AUTOCONNECT__AUTO_PING_INTERVAL" />auto_ping_interval | AUTOCONNECT__AUTO_PING_INTERVAL | num | 300 | Seconds between server generated pings to the client |
| <span id="AUTOCONNECT__AUTO_PING_TIMEOUT" />auto_ping_timeout | AUTOCONNECT__AUTO_PING_TIMEOUT | num | 4 | Seconds to wait for a ping response from the client |
| <span id="AUTOCONNECT__CRYPTO_KEY" />crypto_key | AUTOCONNECT__CRYPTO_KEY | string | _internally generated value_ | The cryptographic key to use for endpoint encryption |
| <span id="AUTOCONNECT__DB_DSN" />db_dsn | AUTOCONNECT__DB_DSN | string | _None_ | Data Source Name for the back end database |
| <span id="AUTOCONNECT__DB_SETTINGS" />db_settings | AUTOCONNECT__DB_SETTINGS | string | _None_ | Serialized JSON structure of Database settings (see each data store for details)
| <span id="AUTOCONNECT__ENDPOINT_HOSTNAME" />endpoint_hostname | AUTOCONNECT__ENDPOINT_HOSTNAME | string | "localhost" | The hostname for the endpoint URL |
| <span id="AUTOCONNECT__ENDPOINT_PORT" />endpoint_port | AUTOCONNECT__ENDPOINT_PORT | num | 8082 | Optional port override for the endpoint URL |
| <span id="AUTOCONNECT__ENDPOINT_SCHEME" />endpoint_scheme | AUTOCONNECT__ENDPOINT_SCHEME | string | "http" | The URL scheme (http/https) for the endpoint URL |
| <span id="AUTOCONNECT__HOSTNAME" />hostname | AUTOCONNECT__HOSTNAME | string | _None_ | The machine host name (e.g. `localhost`) |
| <span id="AUTOCONNECT__HUMAN_LOGS" />human_logs | AUTOCONNECT__HUMAN_LOGS | bool | false | Whether to use human readable log formatting (e.g. timestamps, log levels) |
| <span id="AUTOCONNECT__MEGAPHONE_API_TOKEN" />megaphone_api_token | AUTOCONNECT__MEGAPHONE_API_TOKEN | string | _None_ | Access token for the Remote Settings "Megaphone" API server |
| <span id="AUTOCONNECT__MEGAPHONE_API_URL" />megaphone_api_url | AUTOCONNECT__MEGAPHONE_API_URL | string | _None_ | URL to the Remote Settings "Megaphone" API server |
| <span id="AUTOCONNECT__MEGAPHONE_POLL_INTERVAL" />megaphone_poll_interval | AUTOCONNECT__MEGAPHONE_POLL_INTERVAL | string | _None_ | Period in seconds to poll the Remote Settings "Megaphone" server |
| <span id="AUTOCONNECT__MSG_LIMIT" />msg_limit | AUTOCONNECT__MSG_LIMIT | num | 1000 | Maximum number of messages to store per client |
| <span id="AUTOCONNECT__OPEN_HANDSHAKE_TIMEOUT" />open_handshake_timeout | AUTOCONNECT__OPEN_HANDSHAKE_TIMEOUT | num | 5 | Seconds to wait for a valid `Hello` message from the client |
| <span id="AUTOCONNECT__PORT" />port | AUTOCONNECT__PORT |num | 8080 | The application port to listen on |
| <span id="AUTOCONNECT__RELIABILITY_DSN" />reliability_dsn | AUTOCONNECT__RELIABILITY_DSN | string | _None_ | Data Source Name for the reliability data store, if `--features=reliable_report` enabled |
| <span id="AUTOCONNECT__RELIABILITY_RETRY_COUNT" />reliability_retry_count | AUTOCONNECT__RELIABILITY_RETRY_COUNT | num | 3 | Number of times to retry a Redis transaction write for reliability, if `--features=reliable_report` enabled |
| <span id="AUTOCONNECT__RESOLVE_HOSTNAME" />resolve_hostname | AUTOCONNECT__RESOLVE_HOSTNAME | bool | false | Use the internal IP address of the given hostname |
| <span id="AUTOCONNECT__ROUTER_HOSTNAME" />router_hostname | AUTOCONNECT__ROUTER_HOSTNAME | string | _None_ | Hostname to use for internode communication. _*NOTE*_: This is name or address of the local machine and will be provided internally to other autopush nodes. This will default to the `hostname` value. You only need to set this if the host name to use for internode communication is different. (e.g. you may wish to use the node IP address instead of the hostname if you do not have a DNS
entry for the hostname) |
| <span id="AUTOCONNECT__ROUTER_PORT" />router_port | AUTOCONNECT__ROUTER_PORT | num | 8081 | Router port for autoconnect internode communication. _*NOTE*_: Be sure that this port is accessible to all machines in the Autopush cluster (both autoendpoint and autoconnect). This port does NOT need to be publicly accessible. |
| <span id="AUTOCONNECT__STATSD_HOST" />statsd_host | AUTOCONNECT__STATSD_HOST | string | "localhost" | Name of the statsd collector host |
| <span id="AUTOCONNECT__STATSD_PORT" />statsd_port | AUTOCONNECT__STATSD_PORT | port | 8125 | Port for the statsd collector host |
| <span id="AUTOCONNECT__STATSD_LABEL" />statsd_label | AUTOCONNECT__STATSD_LABEL | string | "autoconnect" | Label to use for statsd metrics |

## Autoendpoint

The following configuration options can be specified either in the configuration file or by using their environment variable form.

| **option** | **env var** | **type** | **default value** | **description** |
|---|---|---|---|---|
| <span id="AUTOEND__SCHEME" />scheme | AUTOEND__SCHEME | string | "http" | The URL scheme (http/https) for the endpoint URL |
| <span id="AUTOEND__HOST" />host | AUTOEND__HOST | string | "127.0.0.1" | The machine host for the endpoint URL (e.g. `localhost`) |
| <span id="AUTOEND__PORT" />port | AUTOEND__PORT | num | 8000 | The application port override for the endpoint URL |
| <span id="AUTOEND__ENDPOINT_URL" />endpoint_url | AUTOEND__ENDPOINT_URL | string | _None_ | The full URL for the endpoint (overrides `scheme`, `host`, and `port` if set) |
| <span id="AUTOEND__DB_DSN" />db_dsn | AUTOEND__DB_DSN | string | _None_ | Data Source Name for the back end database |
| <span id="AUTOEND__DB_SETTINGS" />db_settings | AUTOEND__DB_SETTINGS | string | _None_ | Serialized JSON structure of Database settings (see each data store for details) |
| <span id="AUTOEND__ROUTER_TABLE_NAME" />router_table_name | AUTOEND__ROUTER_TABLE_NAME | string | "router" | The name of the database table to use for the router (Note, this is legacy and will be removed in a future release) |
| <span id="AUTOEND__MESSAGE_TABLE_NAME" />message_table_name | AUTOEND__MESSAGE_TABLE_NAME | string | "message" | The name of the database table to use for messages (Note, this is legacy and will be removed in a future release) |
| <span id="AUTOEND__TRACKING_KEYS" />tracking_keys | AUTOEND__TRACKING_KEYS | string | _None_ | Comma separated list of VAPID public keys to track for reliability (if `--features=reliable_report` enabled) |
| <span id="AUTOEND__MAX_DATA_BYTES" />max_data_bytes | AUTOEND__MAX_DATA_BYTES | num | 4096 | Maximum number of bytes to accept in the `data` field of a message |
| <span id="AUTOEND__CRYPTO_KEYS" />crypto_keys | AUTOEND__CRYPTO_KEYS | string | _internally generated value_ | Comma separated list of cryptographic keys to use for endpoint encryption (the first key will be used for encrypting new messages) |
| <span id="AUTOEND__AUTH_KEYS" />auth_keys | AUTOEND__AUTH_KEYS | string | _None_ | Comma separated list of authentication keys to use for client endpoint authentication (the first key will be used for authenticating new messages) |
| <span id="AUTOEND__HUMAN_LOGS" />human_logs | AUTOEND__HUMAN_LOGS | bool | false | Whether to use human readable log formatting (e.g. timestamps, log levels) |
| <span id="AUTOEND__CONNECTION_TIMEOUT_MILLIS" />connection_timeout_millis | AUTOEND__CONNECTION_TIMEOUT_MILLIS | num | 1000 | Number of milliseconds to wait for a bridge connection before timing out |
| <span id="AUTOEND__REQUEST_TIMEOUT_MILLIS" />request_timeout_millis | AUTOEND__REQUEST_TIMEOUT_MILLIS | num | 3000 | Number of milliseconds to wait for a bridge request before timing out |
| <span id="AUTOEND__STATSD_HOST" />statsd_host | AUTOEND__STATSD_HOST | string | "localhost" | Name of the statsd collector host |
| <span id="AUTOEND__STATSD_PORT" />statsd_port | AUTOEND__STATSD_PORT | port | 8125 |
| <span id="AUTOEND__STATSD_LABEL" />statsd_label | AUTOEND__STATSD_LABEL | string | "autoendpoint" | Label to use for statsd metrics |
| <span id="AUTOEND__RELIABILITY_DSN" />reliability_dsn | AUTOEND__RELIABILITY_DSN | string | _None_ | Data Source Name for the reliability data store, if `--features=reliable_report` enabled |
| <span id="AUTOEND__RELIABILITY_RETRY_COUNT" />reliability_retry_count | AUTOEND__RELIABILITY_RETRY_COUNT | num | 10 | Number of times to retry a Redis transaction write for reliability, if `--features=reliable_report` enabled |
| <span id="AUTOEND__MAX_NOTIFICATION_TTL" />max_notification_ttl | AUTOEND__MAX_NOTIFICATION_TTL | num | 2592000 (30 days) | Maximum allowed TTL (in seconds) for a notification message |

### FCM Settings
| **option** | **env var** | **type** | **default value** | **description** |
|---|---|---|---|---|
| <span id="AUTOEND__FCM__SERVER_CREDENTIALS" />fcm.server_credentials | AUTOEND__FCM__SERVER_CREDENTIALS | string | _None_ | Serialized JSON structure of FCM server credentials (see below) |
| <span id="AUTOEND__FCM__MAX_DATA" />fcm.max_data | AUTOEND__FCM__MAX_DATA | num | 4096 | Maximum number of bytes to accept in the `data` field of a message for FCM messages |
| <span id="AUTOEND__FCM__BASE_URL" />fcm.base_url | AUTOEND__FCM__BASE_URL | string | "https://fcm.googleapis.com/fcm/send" | The base URL for FCM API requests |
| <span id="AUTOEND__FCM__TIMEOUT" />fcm.timeout | AUTOEND__FCM__TIMEOUT | num | 3 | Number of seconds to wait for an FCM API request before timing out |
| <span id="AUTOEND__FCM__MAX_TTL" />fcm.max_ttl | AUTOEND__FCM__MAX_TTL | num | 2419200 (28 days) | Maximum allowed TTL (in seconds) for an FCM message |

`fcm.server_credentials` should be a serialized JSON dictionary. Each key should be the `app_id` of the FCM channel as identified by the client. This `app_id` would appear in the registration request URL (e.g. the "foo" channel would appear in the registration as `/v1/fcm/foo/registration/...`) with the following structure:

```json
{
    "<app_id>":{
        "project_id": "your-project-id",
        "is_gcm": false,
        "server_access_token": "token-for-fcm-http-v1-api"
    }
}
```
### APNS Settings

| **option** | **env var** | **type** | **default value** | **description** |
|---|---|---|---|---|
| <span id="AUTOEND__APNS__CHANNELS" />apns.channels | AUTOEND__APNS__CHANNELS | string | _None_ | JSON structure of APNS channel definitions (see below) |
| <span id="AUTOEND__APNS__MAX_DATA" />apns.max_data | AUTOEND__APNS__MAX_DATA | num | 4096 | Maximum number of bytes to accept in the `data` field of a message for APNS messages |
| <span id="AUTOEND__APNS__REQUEST_TIMEOUT_SECS" />apns.request_timeout_secs | AUTOEND__APNS__REQUEST_TIMEOUT_SECS | num | 3 | Number of seconds to wait for an APNS API request before timing out |
| <span id="AUTOEND__APNS__POOL_IDLE_TIMEOUT_SECS" />apns.pool_idle_timeout_secs | AUTOEND__APNS__POOL_IDLE_TIMEOUT_SECS | num | 60 | Number of seconds to keep idle connections in the APNS connection pool before closing them |

`apns.channels` should be a serialized JSON structure containing a dictionary. Each key should be the `app_id` of the APNS channel as identified by the client. This `app_id` would appear in the registration request URL (e.g. the "foo" channel would appear in the registration as `/v1/apns/foo/registration/...`) with the following structure:

```json
{
  "<app_id>":
    {
      "cert": "<(path-to-pem-encoded-certificate> | PEM-encoded-certificate)>",
      "key": "<(path-to-pem-encoded-key> | PEM-encoded-key)>",
      "is_sandbox": false,
      "topic": "<com.yourcompany.yourapp>"
    }
}
```

