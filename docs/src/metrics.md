# Delivery Metrics

Autopush reports aggregate metrics that describe how quickly and how reliably
notifications are delivered. They cover all traffic and never identify
individual messages, subscriptions, or users.

There are three delivery metrics:

- [`autopush_delivery_seconds`](#autopush_delivery_seconds): how long delivery
  took, from receipt at autoendpoint.
- [`autopush_stored_delivery_after_connect_seconds`](#autopush_stored_delivery_after_connect_seconds):
  how long a stored message took to deliver once the client was connected.
- [`notification.outcome`](#notificationoutcome): the final result of each
  message, and who was responsible when it was not delivered.

## Where metrics are published

The two latency metrics are Prometheus histograms. The outcome counter is a
statsd counter, sent to the host configured by `statsd_host`.

| Service      | Prometheus endpoint                                                                                    | statsd prefix                          |
| ------------ | ------------------------------------------------------------------------------------------------------ | -------------------------------------- |
| autoendpoint | `GET /metrics` on the main HTTP port                                                                   | `statsd_label`, default `autoendpoint` |
| autoconnect  | `GET /metrics` on the internal [router port](config_options.md#AUTOCONNECT__ROUTER_PORT), default 8081 | `statsd_label`, default `autoconnect`  |

`/metrics` responses use the OpenMetrics text format. On autoconnect, the
endpoint is served only on the router port, never on the public WebSocket port.

autoendpoint serves `/metrics` on the same port as the public API. If
autoendpoint is reachable from the internet, block `/metrics` at your proxy,
load balancer or WAF, and scrape it from inside your network.

A histogram series with labels only appears after its first observation, so a
freshly started node may serve an empty report.

## Latency histograms

Both histograms use the same buckets, in seconds, on both services, so series
can be summed across a whole cluster and compared over any time range. The
buckets are log spaced from 10ms up to the 30 day maximum notification TTL:

```text
0.01, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30, 60, 300, 900, 1800, 3600,
10800, 21600, 43200, 86400, 172800, 345600, 604800, 1209600, 2592000
```

Changing these buckets would break comparison with historical data.

Durations are measured between clocks on different hosts. A negative duration
caused by clock skew is recorded as 0.

### `autopush_delivery_seconds`

The time from when autoendpoint received a notification to when it was
delivered.

| Label   | Values              | Meaning                                                                       |
| ------- | ------------------- | ----------------------------------------------------------------------------- |
| `path`  | `ws`, `fcm`, `apns` | The router that delivered the message. Legacy GCM subscriptions report `fcm`. |
| `route` | `direct`, `stored`  | Whether the message was read from storage before delivery.                    |

When delivery is considered complete depends on the path:

| `path`        | `route`  | Start                                    | End                            | Reported by  |
| ------------- | -------- | ---------------------------------------- | ------------------------------ | ------------ |
| `ws`          | `direct` | Receipt at autoendpoint, in milliseconds | The client ACKs the message    | autoconnect  |
| `ws`          | `stored` | Receipt at autoendpoint, in seconds      | The client ACKs the message    | autoconnect  |
| `fcm`, `apns` | `direct` | Receipt at autoendpoint, in milliseconds | The bridge accepts the message | autoendpoint |

A WebSocket message counts as `stored` whenever it was read back from storage.
That includes messages that were sent directly but stored again because the
client disconnected, or the node shut down, before the client ACKed them.

Stored messages only keep their receipt time to the second, so `stored`
latencies can be up to one second higher than the true value.

For bridged messages, delivery ends when FCM or APNs accepts the message. The
time the bridge takes to reach the device is not included.

### `autopush_stored_delivery_after_connect_seconds`

The time a stored WebSocket message took to deliver once it could be delivered.
That is from the later of when the client connected and when the message was
received, to the client's ACK. It has no labels.

This isolates the part of stored-message latency that autopush controls. The
difference between this histogram and `autopush_delivery_seconds{route="stored"}`
is, in aggregate, the time devices spent offline.

## `notification.outcome`

A statsd counter, incremented once when a message reaches its final state.

| Tag       | Values                                                                                |
| --------- | ------------------------------------------------------------------------------------- |
| `path`    | `ws`, `fcm`, `apns`, or `unknown` when the request failed before its router was known |
| `outcome` | See the table below                                                                   |
| `cause`   | `none`, `client`, `bridge`, `push`                                                    |

| `outcome`        | `cause`  | Meaning                                                                                                                                                                                                                                                          |
| ---------------- | -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `delivered`      | `none`   | The client ACKed the message, or the bridge accepted it.                                                                                                                                                                                                         |
| `expired`        | `none`   | The message had a TTL of 0 and could not be delivered directly, or the message had expired by the time it was read from storage. Note that this does NOT count messages that silently expire in storage and are garbage-collected without attempting to be read. |
| `not_found`      | `client` | The endpoint token could not be decrypted (HTTP 404).                                                                                                                                                                                                            |
| `gone`           | `client` | The subscription or user no longer exists, or the bridge reported the device token as unregistered (HTTP 410).                                                                                                                                                   |
| `bridge_error`   | `bridge` | Any other FCM or APNs error. The reason is reported by `notification.bridge.error`.                                                                                                                                                                              |
| `storage_error`  | `push`   | The message could not be saved to storage.                                                                                                                                                                                                                       |
| `internal_error` | `push`   | Any other server error.                                                                                                                                                                                                                                          |

Some cases are deliberately not counted:

- **Sender errors.** Other 4xx responses, such as invalid headers, VAPID errors,
  an oversized payload, or an unknown API version in the URL, describe the
  sender's request rather than push or the recipient.
- **Messages that expire unread.** A stored message that expires without ever
  being read from storage has no final event to count.

Any ACK counts as `delivered`, whatever its code. Codes 101 (decryption error)
and 102 (not delivered) describe what the client did after it received the
message. Errors raised by the web app's service worker are reported separately
as `ua.command.nack`, tagged with the NACK `code`. A NACK can follow an ACK for
the same message, so NACKs are not outcomes.

## Example queries

These examples are PromQL, and assume the statsd counter is exported to
Prometheus as `notification_outcome`. Adjust the names to match your setup.

The 99th percentile delivery latency by path and route:

```promql
histogram_quantile(0.99,
  sum by (le, path, route) (rate(autopush_delivery_seconds_bucket[5m])))
```

Mean delivery latency, which stays exact over any time range:

```promql
sum by (path, route) (rate(autopush_delivery_seconds_sum[1h]))
  / sum by (path, route) (rate(autopush_delivery_seconds_count[1h]))
```

The share of messages delivered:

```promql
sum(rate(notification_outcome{outcome="delivered"}[1h]))
  / sum(rate(notification_outcome[1h]))
```

Non-delivery by who was responsible:

```promql
sum by (cause, outcome) (rate(notification_outcome{outcome!="delivered"}[1h]))
```

Mean time clients are offline before retrieving messages (only for eventually delivered, non-expired messages):

```promql
(
  sum(rate(autopush_delivery_seconds_sum{path="ws",route="stored"}[1h]))
  - sum(rate(autopush_stored_delivery_after_connect_seconds_sum[1h]))
)
/ sum(rate(autopush_delivery_seconds_count{path="ws",route="stored"}[1h]))
```
