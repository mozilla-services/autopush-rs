# APNS cert update utility scripts.

Every year, we will need to update the APNs certificate. See the
Autopush Operations Manual for additional details. This directory
contains some tooling that has been useful for this process.

## For folk that are interested in running your own server

First off, you're going to need a custom UA in order to accept APNs messages.
Refer to the [Apple APNs documentation](https://developer.apple.com/documentation/usernotifications/sending-notification-requests-to-apns) for details.

The short form version is that you're going to need to follow the steps outlined in the [Apple Documentation](https://developer.apple.com/documentation/usernotifications/establishing-a-certificate-based-connection-to-apns/#Obtain-a-provider-certificate-from-Apple).

A few extra notes:

* Apple currently only accepts keys and certs that are RSA 2048. That may change in the future.
* You need to make sure that you generate a cert for each App ID / Bundle ID you've defined.
* Once you have your cert, convert it to PEM format, inline the new lines (See `convert_cert_to_secret.bash`) and include them in the autoendpoint configuration file:

```yaml
"AUTOEND__CRYPTO_KEYS": ["..."],
"AUTOEND__AUTH_KEYS": ["..."]
"AUTOEND__APNS__CHANNELS": {
    "dev":{
        "topic":"org.mozilla.ios.Fennec",
        "cert":"-----BEGIN CERTIFICATE-----\nAbC...123==\n-----END CERTIFICATE-----\n",
        "key":"-----BEGIN PRIVATE KEY------\naBc...456=\n-----END PRIVATE KEY-----\n",
        },
    ...
},
...
```
(For clarity, the fields have been broken into multiple lines. In the actual config file, they should be on a single line.)
