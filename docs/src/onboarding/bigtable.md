# Bigtable data model

Autopush uses one Bigtable table, conventionally named `autopush`, with four
column families. Router rows, message rows, and reliability rows share the same
table and row-key namespace.

Consequently, `table_bytes_used` is **not** the size of the live offline queue.
It includes every family, acknowledged rows awaiting TTL, expired cells awaiting
compaction, and stale routing data.

## Exactly what is persisted

This is the complete high-level inventory. The detailed schemas and lifecycle
operations follow below.

| Persisted object | When it is written | Exact application data saved | Not saved there |
|---|---|---|---|
| Desktop router row | First channel registration, then connection/channel updates | UAID in the row key; router type; most recent internal `node_id`; connection timestamp; storage cursor; model/optimistic-lock versions; one empty `chid:*` cell per channel | Live socket object, browser/OS, IP address, VAPID key/JWT, notification body |
| Mobile router row | FCM/APNs registration and management | UAID in the row key; router type; native FCM/APNs token; configured app/release-channel data; APNs topic/default `aps` data where supplied; versions; channel cells | Mobile message body, provider delivery receipt, exact browser/OS |
| Ordinary desktop message row | Offline/direct failure, or recovery of an unACKed direct message | UAID, sort timestamp and CHID in the row key; TTL; original publication time; opaque message ID; optional encryption headers; optional Base64URL ciphertext; optional reliability fields | Plaintext, publisher VAPID private key/JWT, browser/OS, ACK history |
| Topic desktop message row | Same fallback/recovery path when `Topic` is set | Same message cells; UAID, CHID and Topic are in the stable row key so a later pending message replaces it | Same exclusions as an ordinary message |
| Reliability row | Only for sampled/tracked messages when reliability reporting is enabled | Random reliability ID in the row key; one qualifier per recorded state; transition time as the cell value | UAID, CHID, message body, browser/OS, publisher identity |

Bigtable itself also stores a family name, qualifier, version, and timestamp for
every cell. Autopush deliberately sets cell timestamps into the future to drive
message and reliability garbage collection. Those storage-level timestamps are
separate from the `timestamp` value saved inside a message row.

### Synthetic examples

The logical contents look approximately like this; these are illustrative IDs,
not an export format:

```text
row 4d07c30a9b20440ab31047d42a3f2c68
  router:router_type       = "webpush"
  router:node_id           = "http://autoconnect-node:8081"
  router:connected_at      = <milliseconds>
  router:current_timestamp = <milliseconds>
  router:record_version    = <integer>
  router:version           = <uuid>
  router:chid:01234567-89ab-4cde-8f01-23456789abcd = <empty>

row 4d07c30a9b20440ab31047d42a3f2c68#02:1784058000123:01234567-89ab-4cde-8f01-23456789abcd
  message:ttl       = <seconds>
  message:timestamp = <publication time in seconds>
  message:version   = <opaque message id>
  message:headers   = <optional encryption metadata JSON>
  message:data      = <optional Base64URL ciphertext>
```

A direct desktop message that is ACKed normally has no message row at all. A
direct message gains one only if delivery falls back to storage or disconnect
recovery saves it. Mobile message bodies never use either Bigtable message
family.

## Row families and keys

The family names below are constants in
`autopush-common/src/db/bigtable/bigtable_client/mod.rs`. Although older
configuration structures still contain family-name fields, current read and
write code uses these fixed names.

| Family | Row key | Purpose |
|---|---|---|
| `router` | `{uaid-simple}` | One user-agent record with routing fields and all subscription channels. |
| `message` | `{uaid-simple}#02:{millisecond-sort-timestamp}:{hyphenated-chid}` | One ordinary stored desktop message. |
| `message_topic` | `{uaid-simple}#01:{hyphenated-chid}:{topic}` | One replaceable stored desktop message for a UAID/CHID/Topic tuple. |
| `reliability` | `{reliability-id}` | Optional, sampled message-state transition history with no payload or UAID field. |

Here, `simple` means the UUID's 32 lower-case hexadecimal characters without
hyphens. The topic key is deliberately stable so that another publication with
the same UAID, CHID, and Topic overwrites the pending topic row.

## Router rows

The `router` family may contain:

| Qualifier | Value |
|---|---|
| `connected_at` | Millisecond timestamp used to resolve concurrent connection updates. |
| `router_type` | `webpush`, `fcm`, `apns`, or a legacy/test value. |
| `router_data` | JSON. For mobile this includes the native token and app/release-channel data. Usually absent for desktop. |
| `node_id` | Internal URL of the autoconnect process that most recently registered the desktop UAID. It may be stale. |
| `current_timestamp` | Cursor after the last fully acknowledged ordinary-message batch. |
| `record_version` | Data-model version. |
| `version` | Random UUID used for optimistic conditional updates. |
| `chid:{hyphenated-uuid}` | Set membership for each subscription channel; the value is empty. |

`get_user` reads this row and also parses all `chid:*` cells into a private
in-memory set. `get_channels` currently reads the same row again with a
qualifier filter. That duplication is intentional in the generic database API
but is unnecessary for the Bigtable representation.

### Intended router TTL versus effective GC

Router writes assign every cell a Bigtable timestamp of approximately
`now + max_router_ttl`, which defaults to 60 days. Hello and authenticated
mobile refresh operations rewrite the row with a fresh timestamp.

However, the repository setup script and the production infrastructure audited
with this guide configure `router` with `maxversions=1` and no maximum-age rule.
The future timestamp therefore does not expire the row by age. The source even
describes behavior for “when router TTLs are eventually enabled.”

At present, stale router records can persist until an explicit `remove_user`.
Do not enable router age GC without first deciding what a desktop profile that
returns after the chosen inactivity period should experience.

## Message rows

Both message families use the same cells:

| Qualifier | Value |
|---|---|
| `ttl` | Effective TTL in seconds after service-level capping. |
| `timestamp` | Original publication time in Unix seconds. |
| `version` | Opaque Fernet message ID returned in the publisher `Location` and sent to Firefox. |
| `headers` | Optional JSON containing content encoding and encryption metadata. |
| `data` | Optional Base64URL representation of the already encrypted body. |
| `reliability_id`, `reliable_state` | Optional fields when reliability reporting is compiled/enabled. |

The body is ciphertext before it reaches Autopush. Base64URL encoding it for
JSON transport/storage increases its size by roughly one third before
Bigtable's own compression and cell overhead are considered.

### Message expiry mechanism

When saving a message, Autopush gives every cell a Bigtable timestamp in the
future: approximately `write time + message TTL`. The `message` and
`message_topic` families use this union GC policy:

```text
max age 1 second OR max versions 1
```

The one-second age is not the user-visible queue lifetime. Because the cell
timestamp is in the future, the cell becomes GC-eligible about one second after
that future timestamp. Reads also filter out cells whose timestamp is earlier
than “now,” so logically expired messages stop appearing before physical
compaction necessarily removes their bytes.

Bigtable garbage collection is asynchronous. Physical table size can include
expired data for days after it is no longer readable by the application.

### A subtle recovery detail

An ordinary first-time store occurs immediately after publication, so `write
time + TTL` and `publication time + TTL` are almost equal. When an unacknowledged
direct message is recovered during a much later disconnect, `save_message`
again uses `now + TTL` for the Bigtable cell timestamp while retaining the
original publication timestamp inside the row.

Autoconnect's logical expiry check still uses `publication timestamp + TTL`,
but the physical cells can remain until a full TTL after the recovery write.
This does not normally change delivery semantics, but it can extend physical
retention and storage usage.

## Reliability rows

When `reliable_report` is enabled for a tracked message, the reliability ID is
the row key. Each state name is a qualifier and its value is the transition's
millisecond timestamp. These rows do not contain the notification body, UAID,
or publisher identity.

Reliability cells are written with a timestamp 60 days in the future. With the
same one-second-age pattern, their intended retention is about 60 days plus
Bigtable compaction delay.

## Reads and writes by lifecycle stage

| Lifecycle event | Bigtable operations |
|---|---|
| Existing desktop Hello | `get_user`; conditional `update_user` rewrites router fields/channels with new `node_id` and `connected_at`; then message-range reads begin. |
| New desktop Hello | No immediate persistent write; user is deferred until first Register. |
| Desktop Register / Unregister | Add or conditionally delete one `chid:*` cell. Unregister does not delete that channel's pending message rows. |
| Mobile registration | Add router user, then add channel. |
| Mobile token update / daily channel check | Read router user; update router data or refresh the whole router row; read channels for the channel response. |
| Every publication | `get_user`; desktop additionally calls `get_channels`. |
| Desktop direct delivery | No message-row write initially; router reads already happened. |
| Desktop fallback | `save_message`, then `get_user` again to detect a reconnect. |
| Desktop reconnect/check-storage | Range-read `message_topic`, then range-read `message`, filtered by current time and `current_timestamp`. |
| ACK stored topic | Delete the message row. |
| ACK stored ordinary batch | Write `current_timestamp` and a new router-row version; ordinary message rows remain. |
| Disconnect before direct ACK | Write unacknowledged non-zero-TTL messages; read router row and possibly notify a newer node. |
| Publisher cancellation | Decode `Location` message ID and delete its message row. |
| User/channel removal | User removal deletes the plain router row only; channel removal deletes one `chid:*` cell. Prefixed message rows remain until message GC or explicit deletion. |

## Logical queue versus physical bytes

An ordinary message can be in one of these states:

1. Direct and only in autoconnect memory.
2. Direct and ACKed, therefore absent from Bigtable.
3. Stored and not yet sent.
4. Stored, sent, but not yet ACKed.
5. Stored and ACKed; hidden by `current_timestamp` but still physically present.
6. TTL-expired and hidden by read filters, but waiting for compaction.

Only states 3 and 4 are logically pending. Bigtable's table-size metric also
counts states 5 and 6, plus router, topic, and reliability data. That is why it
must not be labelled “undelivered messages” without a separate family/row
analysis.

## What Bigtable cannot tell you

The table does not persist:

- the exact desktop browser or OS;
- a record for each currently open socket;
- proof that `node_id` still has a live connection;
- end-device receipt for mobile messages;
- plaintext notification content;
- a direct message that was ACKed without ever needing recovery.

## Code and configuration map

| Concern | Primary source |
|---|---|
| Bigtable settings | `autopush-common/src/db/bigtable/mod.rs` |
| Row serialization and all operations | `autopush-common/src/db/bigtable/bigtable_client/mod.rs` |
| Logical user model | `autopush-common/src/db/mod.rs` |
| Logical notification model | `autopush-common/src/notification.rs` |
| Local family creation/GC | `scripts/setup_bt.sh` |
