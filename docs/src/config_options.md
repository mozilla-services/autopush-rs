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
| actix_max_connections | AUTOCONNECT__ACTIX_MAX_CONNECTIONS | num | _None_ | Maximum number of concurrent connections for the Actix web server |
| actix_workers | AUTOCONNECT__ACTIX_WORKERS | num | _None_ | Number of worker threads for the Actix web server per bind address |
| auto_ping_interval | AUTOCONNECT__AUTO_PING_INTERVAL | num | 300 | Seconds between server generated pings to the client |
| auto_ping_timeout | AUTOCONNECT__AUTO_PING_TIMEOUT | num | 4 | Seconds to wait for a ping response from the client |
| crypto_key | AUTOCONNECT__CRYPTO_KEY | string | _internally generated value_ | The cryptographic key to use for endpoint encryption |
| db_dsn | AUTOCONNECT__DB_DSN | string | _None_ | Data Source Name for the back end database |
| db_settings | AUTOCONNECT__DB_SETTINGS | string | _None_ | Serialized JSON structure of Database settings (see each data store for details)
| endpoint_hostname | AUTOCONNECT__ENDPOINT_HOSTNAME | string | "localhost" | The hostname for the endpoint URL |
| endpoint_port | AUTOCONNECT__ENDPOINT_PORT | num | 8082 | Optional port override for the endpoint URL |
| endpoint_scheme | AUTOCONNECT__ENDPOINT_SCHEME | string | "http" | The URL scheme (http/https) for the endpoint URL |
| hostname | AUTOCONNECT__HOSTNAME | string | _None_ | The machine host name (e.g. `localhost`) |
| human_logs | AUTOCONNECT__HUMAN_LOGS | bool | false | Whether to use human readable log formatting (e.g. timestamps, log levels) |
| megaphone_api_token | AUTOCONNECT__MEGAPHONE_API_TOKEN | string | _None_ | Access token for the Remote Settings "Megaphone" API server |
| megaphone_api_url | AUTOCONNECT__MEGAPHONE_API_URL | string | _None_ | URL to the Remote Settings "Megaphone" API server |
| megaphone_poll_interval | AUTOCONNECT__MEGAPHONE_POLL_INTERVAL | string | _None_ | Period in seconds to poll the Remote Settings "Megaphone" server |
| msg_limit | AUTOCONNECT__MSG_LIMIT | num | 1000 | Maximum number of messages to store per client |
| open_handshake_timeout | AUTOCONNECT__OPEN_HANDSHAKE_TIMEOUT | num | 5 | Seconds to wait for a valid `Hello` message from the client |
| port | AUTOCONNECT__PORT |num | 8080 | The application port to listen on |
| reliability_dsn | AUTOCONNECT__RELIABILITY_DSN | string | _None_ | Data Source Name for the reliability data store, if `--features=reliable_report` enabled |
| reliability_retry_count | AUTOCONNECT__RELIABILITY_RETRY_COUNT | num | 3 | Number of times to retry a Redis transaction write for reliability, if `--features=reliable_report` enabled |
| resolve_hostname | AUTOCONNECT__RESOLVE_HOSTNAME | bool | false | Use the internal IP address of the given hostname |
| router_hostname | AUTOCONNECT__ROUTER_HOSTNAME | string | _None_ | Hostname to use for internode communication |
| router_port | AUTOCONNECT__ROUTER_PORT | num | 8081 | Router port for autoconnect internode communication |
| statsd_host | AUTOCONNECT__STATSD_HOST | string | "localhost" | Name of the statsd collector host |
| statsd_port | AUTOCONNECT__STATSD_PORT | port | 8125 | Port for the statsd collector host |
| statsd_label | AUTOCONNECT__STATSD_LABEL | string | "autoconnect" | Label to use for statsd metrics |

## Autoendpoint

The following configuration options can be specified either in the configuration file or by using their environment variable form.

| **option** | **env var** | **type** | **default value** | **description** |
|---|---|---|---|---|
| scheme | AUTOENDPOINT__SCHEME | string | "http" | The URL scheme (http/https) for the endpoint URL |
| host | AUTOENDPOINT__HOST | string | "127.0.0.1" | The machine host for the endpoint URL (e.g. `localhost`) |
| port | AUTOENDPOINT__PORT | num | 8000 | The application port override for the endpoint URL |
| endpoint_url | AUTOENDPOINT__ENDPOINT_URL | string | _None_ | The full URL for the endpoint (overrides `scheme`, `host`, and `port` if set) |
| db_dsn | AUTOENDPOINT__DB_DSN | string | _None_ | Data Source Name for the back end database |
| db_settings | AUTOENDPOINT__DB_SETTINGS | string | _None_ | Serialized JSON structure of Database settings (see each data store for details) |
| router_table_name | AUTOENDPOINT__ROUTER_TABLE_NAME | string | "router" | The name of the database table to use for the router (Note, this is legacy and will be removed in a future release) |
| message_table_name | AUTOENDPOINT__MESSAGE_TABLE_NAME | string | "message" | The name of the database table to use for messages (Note, this is legacy and will be removed in a future release) |
| tracking_keys | AUTOENDPOINT__TRACKING_KEYS | string | _None_ | Comma separated list of VAPID public keys to track for reliability (if `--features=reliable_report` enabled) |
| max_data_bytes | AUTOENDPOINT__MAX_DATA_BYTES | num | 4096 | Maximum number of bytes to accept in the `data` field of a message |
| crypto_keys | AUTOENDPOINT__CRYPTO_KEYS | string | _internally generated value_ | Comma separated list of cryptographic keys to use for endpoint encryption (the first key will be used for encrypting new messages) |
| auth_keys | AUTOENDPOINT__AUTH_KEYS | string | _None_ | Comma separated list of authentication keys to use for client endpoint authentication (the first key will be used for authenticating new messages) |
| human_logs | AUTOENDPOINT__HUMAN_LOGS | bool | false | Whether to use human readable log formatting (e.g. timestamps, log levels) |
| connection_timeout_millis | AUTOENDPOINT__CONNECTION_TIMEOUT_MILLIS | num | 1000 | Number of milliseconds to wait for a bridge connection before timing out |
| request_timeout_millis | AUTOENDPOINT__REQUEST_TIMEOUT_MILLIS | num | 3000 | Number of milliseconds to wait for a bridge request before timing out |
| statsd_host | AUTOENDPOINT__STATSD_HOST | string | "localhost" | Name of the statsd collector host |
| statsd_port | AUTOENDPOINT__STATSD_PORT | port | 8125 |
| statsd_label | AUTOENDPOINT__STATSD_LABEL | string | "autoendpoint" | Label to use for statsd metrics |
| reliability_dsn | AUTOENDPOINT__RELIABILITY_DSN | string | _None_ | Data Source Name for the reliability data store, if `--features=reliable_report` enabled |
| reliability_retry_count | AUTOENDPOINT__RELIABILITY_RETRY_COUNT | num | 10 | Number of times to retry a Redis transaction write for reliability, if `--features=reliable_report` enabled |
| max_notification_ttl | AUTOENDPOINT__MAX_NOTIFICATION_TTL | num | 2592000 (30 days) | Maximum allowed TTL (in seconds) for a notification message |

### FCM Settings
| **option** | **env var** | **type** | **default value** | **description** |
|---|---|---|---|---|
| fcm.server_credentials | AUTOENDPOINT__FCM__SERVER_CREDENTIALS | string | _None_ | Serialized JSON structure of FCM server credentials (see below) |
| fcm.max_data | AUTOENDPOINT__FCM__MAX_DATA | num | 4096 | Maximum number of bytes to accept in the `data` field of a message for FCM messages |
| fcm.base_url | AUTOENDPOINT__FCM__BASE_URL | string | "https://fcm.googleapis.com/fcm/send" | The base URL for FCM API requests |
| fcm.timeout | AUTOENDPOINT__FCM__TIMEOUT | num | 3 | Number of seconds to wait for an FCM API request before timing out |
| fcm.max_ttl | AUTOENDPOINT__FCM__MAX_TTL | num | 2419200 (28 days) | Maximum allowed TTL (in seconds) for an FCM message |

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
| apns.channels | AUTOENDPOINT__APNS__CHANNELS | string | _None_ | JSON structure of APNS channel definitions (see below) |
| apns.max_data | AUTOENDPOINT__APNS__MAX_DATA | num | 4096 | Maximum number of bytes to accept in the `data` field of a message for APNS messages |
| apns.request_timeout_secs | AUTOENDPOINT__APNS__REQUEST_TIMEOUT_SECS | num | 3 | Number of seconds to wait for an APNS API request before timing out |
| apns.pool_idle_timeout_secs | AUTOENDPOINT__APNS__POOL_IDLE_TIMEOUT_SECS | num | 60 | Number of seconds to keep idle connections in the APNS connection pool before closing them |

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

