# Error Codes

Autopush uses error codes based on [HTTP response
codes](https://www.w3.org/Protocols/rfc2616/rfc2616-sec10.html). An
error response will contain a JSON body including an additional error
information (see `error_resp`).

Unless otherwise specified, all calls return one the following error
statuses:

* 20x - **Success** - The message was accepted for transmission to the
    client. Please note that the message may still be rejected by the
    User Agent if there is an error with the message's encryption.

* 301 - **Moved + \`Location:\`** if `{client_token}` is invalid (Bridge API
    Only) - Bridged services (ones that run over third party services
    like GCM and APNS), may require a new URL be used. Please stop using
    the old URL immediately and instead use the new URL provided.

* 400 - **Bad Parameters** -- One or more of the parameters specified
    is invalid. See the following sub-errors indicated by `errno`

    -   errno 101 - Missing necessary crypto keys - One or more required
        crypto key elements are missing from this transaction. Refer to
        the [appropriate
        specification](https://datatracker.ietf.org/doc/draft-ietf-httpbis-encryption-encoding/)
        for the requested content-type.

    -   errno 108 - Router type is invalid - The URL contains an invalid
        router type, which may be from URL corruption or an unsupported
        bridge. Refer to `bridge_api`.

    -   errno 110 - Invalid crypto keys specified - One or more of the
        crytpo key elements are invalid. Refer to the [appropriate
        specification](https://datatracker.ietf.org/doc/draft-ietf-httpbis-encryption-encoding/)
        for the requested content-type.

    -   errno 111 - Missing Required Header - A required crypto element
        header is missing. Refer to the [appropriate
        specification](https://datatracker.ietf.org/doc/draft-ietf-httpbis-encryption-encoding/)
        for the requested content-type.

         -   Missing TTL Header - Include the Time To Live header
             ([IETF WebPush protocol
             §6.2](https://tools.ietf.org/html/draft-ietf-webpush-protocol#section-6.2))
         -   Missing Crypto Headers - Include the appropriate
             encryption headers ([WebPush Encryption
             §3.2](https://webpush-wg.github.io/webpush-encryption/#rfc.section.3.2)
             and [WebPush VAPID
             §4](https://tools.ietf.org/html/draft-ietf-webpush-vapid-02#section-4))

    -   errno 112 - Invalid TTL header value - The Time To Live "TTL"
        header contains an invalid or unreadable value. Please change to
        a number of seconds that this message should live, between 0
        (message should be dropped immediately if user is unavailable)
        and 2592000 (hold for delivery within the next approximately 30
        days).

    -   errno 113 - Invalid Topic header value - The Topic header
        contains an invalid or unreadable value. Please use only ASCII
        alphanumeric values \[A-Za-z0-9\] and a maximum length of 32
        bytes..

* 401 - **Bad Authorization** - `Authorization` header is invalid or missing.
    See the [VAPID
    specification](https://datatracker.ietf.org/doc/draft-ietf-webpush-vapid/).

    -   errno 109 - Invalid authentication

* 404 - **Endpoint Not Found** - The URL specified is invalid and
    should not be used again.

     -   errno 102 - Invalid URL endpoint

* 410 - **Endpoint Not Valid** - The URL specified is no longer valid
    and should no longer be used. A User has become permanently
    unavailable at this URL.

    -   errno 103 - Expired URL endpoint
    -   errno 105 - Endpoint became unavailable during request
    -   errno 106 - Invalid subscription

* 413 - **Payload too large** - The body of the message to send is too
    large. The max data that can be sent is 4028 characters. Please
    reduce the size of the message.

    -   errno 104 - Data payload too large

* 500 - **Unknown server error** - An internal error occurred within
    the Push Server.

    -   errno 999 - Unknown error

* 502 - **Bad Gateway** - The Push Service received an invalid
    response from an upstream Bridge service.

    -   errno 900 - Internal Bridge misconfiguration
    -   errno 901 - Invalid authentication
    -   errno 902 - An error occurred while establishing a connection
    -   errno 903 - The request timed out

* 503 - **Server temporarily unavaliable.** - The Push Service is
    currently unavailable. See the error number "errno" value to see if
    retries are available.

    -   errno 201 - Use exponential back-off for retries
    -   errno 202 - Immediate retry ok
