# Actors, services, and registration

## Who connects to what?

| Actor | Connects to | Protocol and route | Purpose |
|---|---|---|---|
| Desktop Firefox | autoconnect public listener | WebSocket upgrade on `GET /` | Hold a receiving connection; perform `hello`, subscription registration, ACK/NACK, Ping, and broadcast operations. |
| Firefox mobile push component | autoendpoint | HTTP under `/v1/{router_type}/{app_id}/registration...` | Register or update an FCM/APNs token and manage channels. |
| Application server | autoendpoint | `POST /wpush/{api_version}/{token}` | Publish an encrypted Web Push notification. |
| autoendpoint | autoconnect internal router listener | `PUT /push/{uaid}` or `PUT /notif/{uaid}` | Queue a direct notification on a connection node, or ask that node to read storage. |
| autoendpoint | FCM or APNs | Provider HTTPS API | Hand a mobile notification to the native push provider. |
| autoconnect and autoendpoint | Bigtable | gRPC | Read/write routing, channels, desktop messages, and optional reliability records. |

The publisher is not the receiving client. Its `User-Agent` header, IP address,
and VAPID identity describe the sender and must not be interpreted as the
browser or operating system that will receive the notification.

## Desktop registration

Desktop subscription management uses the WebSocket protocol defined in
`autoconnect/autoconnect-common/src/protocol.rs`.

### 1. Connect and identify

Firefox opens a WebSocket to autoconnect. The HTTP upgrade carries a normal
`User-Agent` header. The first text message must be `hello` and must arrive
within the configured handshake timeout:

```json
{
  "messageType": "hello",
  "uaid": "optional-previous-uaid",
  "channelIDs": [],
  "broadcasts": {}
}
```

The current Rust state machine ignores `channelIDs`; channel membership comes
from the database. If the supplied UAID is malformed or not found, the server
issues a new UAID.

For an existing UAID, autoconnect:

1. Reads the Bigtable router row.
2. Replaces `node_id` with this process's internal router URL.
3. Updates `connected_at` and refreshes the row using optimistic concurrency.
4. Marks the session to check stored notifications.

For a new UAID, the user record is initially held in memory. It is written to
Bigtable lazily when the client first registers a channel. A client that says
Hello but never registers need not create a persistent user row.

The server responds with the accepted UAID, status, broadcast changes, and the
legacy `use_webpush: true` field.

### 2. Register a subscription channel

Firefox sends a lower-case, hyphenated UUID and optionally a VAPID public key:

```json
{
  "messageType": "register",
  "channelID": "01234567-89ab-4cde-8f01-23456789abcd",
  "key": "optional-url-safe-vapid-public-key"
}
```

Autoconnect ensures the user exists, adds a `chid:<uuid>` cell to the router
row, and returns a `pushEndpoint`. The endpoint's Fernet-encrypted token holds:

- v1: the UAID and CHID;
- v2: the UAID, CHID, and SHA-256 digest of the supplied VAPID public key.

It does **not** contain the receiving browser, OS, account, native device token,
or message-decryption key.

The site gives the endpoint to its application server. Future messages enter
Autopush through that URL, not through the WebSocket registration request.

## What VAPID is and is not

**VAPID** means **Voluntary Application Server Identification**. It is the Web
Push mechanism for authenticating the application server that publishes a
notification. Its specification is [RFC 8292](https://www.rfc-editor.org/rfc/rfc8292.html).

The application server owns a P-256 ECDSA key pair:

- The **private key** stays with the publisher and signs a short-lived JWT.
- The **public key** accompanies the publication. In the current VAPID format,
  the `Authorization` header contains both the signed token (`t`) and public key
  (`k`); the older format carries the public key as `p256ecdsa` in
  `Crypto-Key`.
- The JWT contains an expiration and the intended push-service origin
  (`aud`). It may contain a contact URI (`sub`).

Autoendpoint verifies the ES256 signature, audience, expiration, and maximum
one-day token lifetime. If Firefox registered the subscription with a VAPID
public key, Autopush created a v2 endpoint containing that key's SHA-256 digest.
Publication to that endpoint additionally requires the supplied public key to
match the digest. Possessing a copied endpoint URL alone is then insufficient
to publish; the sender must also possess the corresponding private key.

VAPID is separate from notification-content encryption:

| Question | VAPID | Web Push content encryption |
|---|---|---|
| What does it protect? | Who may publish to the subscription. | What the notification says. |
| Who has the private/secret material? | The application server has the VAPID private key. | Publisher and receiving browser use subscription encryption material. |
| Can Autopush decrypt the body? | No; VAPID does not provide a content-decryption key. | No; Autopush only transports the ciphertext and metadata. |

The VAPID private key never enters Autopush. The JWT and public key are
validated for the publication but are not fields in the Bigtable router or
message schemas. The v2 public-key **digest** lives in the encrypted endpoint
token returned at registration, not in the Bigtable router row.

## Mobile registration

Firefox for Android and iOS uses autoendpoint's registration API rather than a
long-lived Autopush WebSocket:

```text
POST /v1/fcm/{app_id}/registration
POST /v1/apns/{app_id}/registration
```

The JSON request includes a native device token and may include a chosen CHID,
a VAPID key, or APNs `aps` defaults. Autoendpoint validates the configured
application ID, creates a UAID and channel, then writes a router row:

- FCM `router_data`: `token` and `app_id`;
- APNs `router_data`: `token`, `rel_channel`, and optional `aps` JSON.

The response includes the UAID, CHID, Web Push endpoint, and an HMAC-based
secret used to authenticate later registration-management requests.

Mobile clients can subsequently:

- `PUT` a replacement native token;
- `GET` the channel list, which also refreshes the router row;
- create or delete channels;
- delete the whole registration.

The native FCM/APNs service is Autopush's external handoff after publication.
Autopush does not place mobile message bodies into its desktop Bigtable message
queue or receive a later device-level ACK.

## Service boundaries in code

| Concern | Primary code |
|---|---|
| Public WebSocket and internal node routes | `autoconnect/autoconnect-web/src/lib.rs`, `routes.rs` |
| WebSocket wire messages | `autoconnect/autoconnect-common/src/protocol.rs` |
| Hello and UAID selection | `autoconnect/.../autoconnect-ws-sm/src/unidentified.rs` |
| Desktop Register/Unregister | `autoconnect/.../identified/on_client_msg.rs` |
| autoendpoint HTTP routes | `autoendpoint/src/server.rs` |
| Mobile registration | `autoendpoint/src/routes/registration.rs` |
| Push endpoint construction | `autopush-common/src/endpoint.rs` |
