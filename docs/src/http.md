# HTTP Endpoints for Notifications

Autopush exposes three HTTP endpoints:

`/wpush/...`

This is tied to the Endpoint Handler
`~autopush.web.webpush.WebPushHandler` This endpoint is returned by the
Push registration process and is used by the `AppServer` to send Push
alerts to the Application. See `send`.

`/m/...`

This is tied to `~autopush.web.message.MessageHandler`. This endpoint
allows a message that has not yet been delivered to be deleted. See
`cancel`.

`/v1/.../.../registration/...`

This is tied to the `reg_calls` Handlers. This endpoint is used by
devices that wish to use `bridging` protocols to register new channels.

*NOTE*: This is not intended to be used by app developers. Please see
the [Web Push API on
MDN](https://developer.mozilla.org/en-US/docs/Web/API/Push_API) for how
to use WebPush. See `bridge_api`.

---

<a id="http-api"> </a>

# Push Service HTTP API

The following section describes how remote servers can send Push
Notifications to apps running on remote User Agents.

## Lexicon

 **{UAID}**
_The Push User Agent Registration ID_

Push assigns each remote recipient a unique identifier. {UAID}s are
UUIDs in lower case, undashed format. (e.g.
'01234567abcdabcdabcd01234567abcd') This value is assigned during
**Registration**

**{CHID}**  
_The `Channel` Subscription ID_

Push assigns a unique identifier for each subscription for a given
{UAID}. Like {UAID}s, {CHID}s are UUIDs, but in lower case, dashed
format( e.g. '01234567-abcd-abcd-abcd-0123456789ab'). The User Agent
usually creates this value and passes it as part of the **Channel
Subscription**. If no value is supplied, the server will create and
return one.

**{message-id}**  
_The unique Message ID_

Push assigns each message for a given Channel Subscription a unique
identifier. This value is assigned during **Send Notification**.

## Response

The responses will be JSON formatted objects. In addition, API calls
will return valid HTTP error codes (see `errors` sub-section for
descriptions of specific errors).

For non-success responses, an extended error code object will be
returned with the following format:

``` json
{
    "code": 404,  // matches the HTTP status code
    "errno": 103, // stable application-level error number
    "error": "Not Found", // string representation of the status
    "message": "No message found" // optional additional error information
}
```

See [Errors](errors.md) for a list of the errors, causes, and potential resolutions.

## Calls

### Send Notification

Send a notification to the given endpoint identified by its `push_endpoint`. Please note, the Push endpoint
URL (which is what is used to send notifications) should be considered
"opaque". We reserve the right to change any portion of the Push URL in
future provisioned URLs.

The `Topic` HTTP header allows new messages
to replace previously sent, unreceived subscription updates. See
`topic`.

**Call:**

```
https://updates.push.services.mozilla.com/wpush/v1/...
```



If the client is using webpush style data delivery, then the body in
its entirety will be regarded as the data payload for the message per
[the WebPush
spec](https://tools.ietf.org/html/draft-thomson-webpush-http2-02#section-5).

> _**Note**_
> Mozilla reserves the right to change the endpoint at any time. Please do not "optimize" by only storing the last token element of the URI. There will be tears.

> _**Note**_
> Some bridged connections require data transcription and may limit the
> length of data that can be sent. For instance, using a GCM/FCM bridge
> will require that the data be converted to base64. This means that
> data may be limited to only 2744 bytes instead of the normal 4096
> bytes.
>

**Reply:**

``` json
{"message-id": {message-id}}
```

**Return Codes:**

- statuscode 404  
  Push subscription is invalid.

- statuscode 202  
 Message stored for delivery to client at a later time.

- statuscode 200  
 Message delivered to node client is connected to.

### Message Topics

Message topics allow newer message content to replace previously sent,
unread messages. This prevents the UA from displaying multiple messages
upon reconnect. [A blog
post](https://hacks.mozilla.org/2016/11/mozilla-push-server-now-supports-topics/)
provides an example of how to use Topics, but a summary is provided
here.

To specify a Topic, include a `Topic` HTTP
header along with your `send`. The topic can be any 32 byte
alpha-numeric string (including "\_" and "-").

Example topics might be `MailMessages`,
`Current_Score`, or `20170814-1400_Meeting_Reminder`

For example:

``` bash
curl -X POST \
    https://push.services.mozilla.com/wpush/abc123... \
    -H "TTL: 86400" \
    -H "Topic: new_mail" \
    -H "Authorization: Vapid AbCd..." \
    ...
```

Would create or replace a message that is valid for the next 24 hours
that has the topic of `new_mail`. The body
of this might contain the number of unread messages. If a new message
arrives, the Application Server could send a second message with a body
containing a revised message count.

Later, when the User reconnects, she will only see a single notification
containing the latest notification, with the most recent new mail
message count.

### Cancel Notification

Delete the message given the `message_id`.

**Call:**

```
https://updates.push.services.mozilla.com/wpush/v1/...
```



**Parameters:**

> None

**Reply:**

``` json
{}
```

**Return Codes:**

See [errors](#error-codes).

---
<a id="bridge-http"> </a>

# Push Service Bridge HTTP Interface

Push allows for remote devices to perform some functions using an HTTP
interface. This is mostly used by devices that are bridging via an
external protocol like
[GCM](https://developers.google.com/cloud-messaging/)/[FCM](https://firebase.google.com/docs/cloud-messaging/)
or
[APNs](https://developer.apple.com/library/ios/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/Introduction.html#//apple_ref/doc/uid/TP40008196-CH1-SW1).
All message bodies must be UTF-8 encoded.

API methods requiring Authorization must provide the Authorization
header containing the registration secret. The registration secret is
returned as "secret" in the registration response.

## Lexicon

For the following call definitions:

**{type}**  
_The bridge type._

  Allowed bridges are `gcm` (Google Cloud
  Messaging), `fcm` (Firebase Cloud
  Messaging), and `apns` (Apple Push
  Notification system)

**{app_id}**  
_The bridge specific application identifier_

Each bridge may require a unique token that addresses the remote
application For GCM/FCM, this is the `SenderID` (or 'project number') and is
pre-negotiated outside of the push service. You can find this number
using the [Google developer
console](https://console.developers.google.com/iam-admin/settings/project).
For APNS, this value is the "platform" or "channel" of development (e.g.
"firefox", "beta", "gecko", etc.) For our examples, we will use a client
token of "33clienttoken33".

**{instance_id}**  
_The bridge specific private identifier token_

Each bridge requires a unique token that addresses the application on a
given user's device. This is the "[Registration
Token](https://firebase.google.com/docs/cloud-messaging/android/client#sample-register)"
for GCM/FCM or "[Device
Token](https://developer.apple.com/library/ios/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/Chapters/IPhoneOSClientImp.html#//apple_ref/doc/uid/TP40008194-CH103-SW2)"
for APNS. This is usually the product of the application registering the
{instance_id} with the native bridge via the user agent. For our
examples, we will use an instance ID of "11-instance-id-11".

**{secret}**  
_The registration secret from the Registration call._

Most calls to the HTTP interface require a Authorization header. The
Authorization header is a simple bearer token, which has been provided
by the **Registration** call and is preceded by the scheme name
"Bearer". For our examples, we will use a registration secret of
"00secret00".

An example of the Authorization header would be:

```html
    Authorization: Bearer 00secret00
```

**{vapidKey}**  
_The [VAPID Key](https://datatracker.ietf.org/doc/html/rfc8292#section-3.2) provided by the subscribing third party_

The VAPID key is optional and provides a way for an application server to voluntarily identify itself.

_*Please Note*_: While the VAPID key is optional, if it is included, the VAPID asserion block _must_ contain a `sub` field containing the publishing contact information as a vaild URI designator. (e.g. `mailto:admin+webpush@example.org` or `https://example.org/contact`). As an example, a minimal VAPID assertion block would contain:

```json
{"aud": "https://updates.push.services.mozilla.com", "exp": 1725468595, "sub": "mailto:admin+webpush@example.com"}
```

Where `exp` and `sub` reflect the expiration time and publishing contact information. The contact information is used in case of an issue with use of the Push service and is never used for marketing purposes.

When the VAPID key is provided, autopush will return an endpoint that can only be used by the application server that provided the key.

**The VAPID key is formatted as a URL-safe Base64 encoded string with no padding.**

## Calls

### Registration

Request a new UAID registration, Channel ID, and set a bridge type and
3rd party bridge instance ID token for this connection. (See
`~autopush.web.registration.NewRegistrationHandler`)

*NOTE*: This call is designed for devices to register endpoints to be
used by bridge protocols. Please see [Web Push
API](https://developer.mozilla.org/en-US/docs/Web/API/Push_API) for how
to use Web Push in your application.

**Call:**

```http
POST /v1/{type}/{appid}/registration
```

This call requires no Authorization header.

**Parameters:**

`{"token":{instance_id},
  "key": {vapidkey}}`

> _**Note**_
> * The VAPID key is optional
> * If additional information is required for the bridge, it may be
> included in the parameters as JSON elements. Currently, no additional
> information is required.
>

**Reply:**

``` json
`{"uaid": {UAID}, "secret": {secret},
"endpoint": "https://updates-push...", "channelID": {CHID}}`
```

example:

``` http
 POST /v1/fcm/33clienttoken33/registration

 {"token": "11-instance-id-11", "key": "AbC12ef0"}
```

``` json
 {"uaid": "01234567-0000-1111-2222-0123456789ab",
 "secret": "00secret00",
 "endpoint": "https://updates-push.services.mozaws.net/push/...",
 "channelID": "00000000-0000-1111-2222-0123456789ab"}
```

**Return Codes:**

See `errors`.

### Token updates

Update the current bridge token value. Note, this is a **\*PUT**\* call,
since we are updating existing information. (See
`~autopush.web.registration.UaidRegistrationHandler`)

**Call:**

```http
  PUT /v1/{type}/{appid}/registration/{uaid}
  Authorization: Bearer {secret}
```

**Parameters:**

```{"token": {instance_id}}```

> _**Note**_
>
>
> If additional information is required for the bridge, it may be
> included in the parameters as JSON elements. Currently, no additional
> information is required.
>

**Reply:**

``` json
{}
```

example:

``` http
 PUT /v1/fcm/33clienttoken33/registration/abcdef012345
 Authorization: Bearer 00secret00

 {"token": "22-instance-id-22"}
```

``` json
{}
```

**Return Codes:**

See `errors`.

### Channel Subscription

Acquire a new ChannelID for a given UAID. (See
`~autopush.web.registration.SubRegistrationHandler`)

**Call:**

```http
  POST /v1/{type}/{app_id}/registration/{uaid}/subscription
  Authorization: Bearer {secret}
```

**Parameters:**

`{key: {vapidKey}}`

> _**Note**_
> VAPID key is optional

**Reply:**

``` json
{"channelID": {CHID}, "endpoint": "https://updates-push..."}
```

example:

``` http
 POST /v1/fcm/33clienttoken33/registration/abcdef012345/subscription
 Authorization: Bearer 00secret00

 {"key": "AbCd01hk"}
```

``` json
 {"channelID": "01234567-0000-1111-2222-0123456789ab",
  "endpoint": "https://updates-push.services.mozaws.net/push/..."}
```

**Return Codes:**

See `errors`.

### Unregister UAID (and all associated ChannelID subscriptions)

Indicate that the UAID, and by extension all associated subscriptions,
is no longer valid. (See
`~autopush.web.registration.UaidRegistrationHandler`)

**Call:**

```http
  DELETE /v1/{type}/{app_id}/registration/{uaid}
  Authorization: Bearer {secret}
```

**Parameters:**

`{}`

**Reply:**

``` json
{}
```

**Return Codes:**

See `errors`.

### Unsubscribe Channel

Remove a given ChannelID subscription from a UAID. (See:
`~autopush.web.registration.ChannelRegistrationHandler`)

**Call:**

```html
  DELETE /v1/{type}/{app_id}/registration/{uaid}/subscription/{CHID}
  Authorization: Bearer {secret}
```

**Parameters:**

`{}`

**Reply:**

``` json
{}
```

**Return Codes:**

See `errors`.

### Get Known Channels for a UAID

Fetch the known ChannelIDs for a given bridged endpoint. This is useful
to check link status. If no channelIDs are present for a given UAID, an
empty set of channelIDs will be returned. (See:
`~autopush.web.registration.UaidRegistrationHandler`)

**Call:**

```http
  GET /v1/{type}/{app_id}/registration/{UAID}/
  Authorization: Bearer {secret}
```

**Parameters:**

`{}`

**Reply:**

``` json
{"uaid": {UAID}, "channelIDs": [{ChannelID}, ...]}
```

example:

``` http
GET /v1/gcm/33clienttoken33/registration/abcdef012345/
Authorization: Bearer 00secret00
{}
```

``` json
 {"uaid": "abcdef012345",
 "channelIDS": ["01234567-0000-1111-2222-0123456789ab", "76543210-0000-1111-2222-0123456789ab"]}
```

**Return Codes:**

See `errors`.
