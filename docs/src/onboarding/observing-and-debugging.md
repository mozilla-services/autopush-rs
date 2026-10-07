# Observing and debugging the lifecycle

The most common debugging mistake is treating an upstream response as proof of
a downstream event. Autopush has several acceptance boundaries.

## What each success proves

| Signal | What it proves | What it does not prove |
|---|---|---|
| Publisher receives `201` | The selected router accepted the publication according to its semantics. | Desktop Firefox displayed or ACKed it. |
| Internal `PUT /push/{uaid}` returns `200` | The autoconnect process queued it in the UAID's bounded in-process channel. | WebSocket write or client ACK. |
| Internal `PUT /notif/{uaid}` returns `200` | The autoconnect process queued a command to check storage. | Bigtable read, WebSocket write, or client ACK. |
| `ua.notification.sent{source=Direct}` | Autoconnect produced a WebSocket notification for the session. | Client processing or ACK. |
| Desktop ACK metric/session counter | Autoconnect matched an ACK to its in-memory notification state. | User-visible display; ACK codes can indicate decryption/not-delivered states when reliability reporting is used. |
| FCM/APNs success | Native provider accepted the request. | Device receipt or application handling. |
| Bigtable `table_bytes_used` | Physical bytes for the whole table. | Number or bytes of logically pending messages. |

## High-value metrics and logs

Metric names are defined in `autopush-common/src/metric_name.rs`, with a few
string-named metrics at their call sites.

| Question | Signals to inspect |
|---|---|
| How many valid publications arrive? | `notification.received`, HTTP status and request-rate panels. |
| Direct versus stored bytes | `notification.message.data{destination=Direct|Stored}`. |
| How many Bigtable message writes? | `notification.message.stored{topic,database}`. |
| Is internal routing stale or overloaded? | `direct.delivery_status{status}`, `direct.delivery_time`, `error.node.timeout`, `error.node.connect`. |
| Are connection nodes writing to Firefox? | `ua.notification.sent{source=Direct|Stored,topic,os}` and `ua.message_data`. |
| Are clients acknowledging? | `ua.command.ack`, `ua.command.nack`, and the final `Session` log counters. |
| Are stored messages being drained? | `notification.message.retrieved{topic}`, `stored_retrieved`, `stored_acked`. |
| Are connections healthy? | `ua.connection.lifespan`, connection count, Hello rate, Pong-timeout/disconnect reasons. |
| Is Bigtable failing? | Bigtable latency/error metrics and `error.bigtable.circuit_breaker_open`. |

The final `Session` log is especially useful. It includes normalized and exact
browser/OS fields, connection duration, direct/stored ACK counts, NACKs,
registrations, recovered direct messages, and a disconnect reason.

## A practical missing-message trace

1. **Publication:** find the autoendpoint request and response. A 4xx generally
   means token, VAPID, TTL/encryption, user, or channel validation failed before
   routing.
2. **Router type:** determine whether the Bigtable router row selected WebPush,
   FCM, or APNs.
3. **Desktop route:** inspect `direct.delivery_status`. Status `200` means the
   process queue accepted it; `404` means that process did not have an
   available registry entry; transport errors can conditionally clear
   `node_id`.
4. **Desktop send:** inspect `ua.notification.sent` and the session log. If the
   connection closed before ACK, expect `direct_storage` and a Bigtable write.
5. **Stored path:** inspect `notification.message.stored`, then retrieval and
   ACK counters on the later connection. Remember that an ordinary ACK advances
   the cursor rather than deleting the row.
6. **Mobile route:** inspect bridge success/error metrics by platform/app ID.
   Autopush has no later device ACK to follow.

## Known observability gaps

- The service does not currently emit a byte-weighted histogram of requested
  or effective message TTL. That makes the impact of a 5/7/30-day cap hard to
  calculate from service metrics alone.
- There is no delivery-age histogram showing how many stored messages are
  actually retrieved after one, five, seven, or thirty days.
- Standard Bigtable table-size metrics do not split physical bytes by column
  family or logical message state.
- A publisher-facing `201` combines direct, stored, provider-accepted, and
  TTL-zero-drop outcomes. Use router metrics rather than the HTTP code alone.
- A reachable autoconnect node returning `404 Client not available` does not
  currently clear `node_id`; only request/transport errors take the conditional
  clear path. This can cause repeated attempts against a stale-but-reachable
  node until the UAID reconnects or another path removes/updates the user.

The first two histograms are the safest way to evaluate a shorter offline
queue. They answer product impact without scanning the production table.

## Integration tests that demonstrate the contract

`tests/integration/test_integration_all_rust.py` contains executable examples:

- `test_basic_delivery` — basic direct delivery and ACK;
- `test_delivery_repeat_without_ack` — redelivery without ACK;
- `test_repeat_delivery_with_disconnect_without_ack` — recovery after a
  disconnect;
- `test_topic_replacement_delivery` — Topic replacement;
- `test_ttl_0_connected` and `test_ttl_0_not_connected` — best-effort TTL zero;
- `test_delete_saved_notification` — publisher cancellation;
- `test_msg_limit` — UAID reset for an excessive backlog;
- `test_internal_endpoints` — autoconnect's internal node routes.
