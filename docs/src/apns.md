# Configuring for the APNS bridge

APNS requires a current [Apple Developer License](https://developer.apple.com/programs/) for the platform or
platforms you wish to bridge to (e.g. iOS, desktop, etc.). Once that
license has been acquired, you will need to create and export a valid
`.p12` type key file. For this document, we
will concentrate on creating an iOS certificate.

## Create the App ID

First, you will need an Application ID. If you do not already have an
application, you will need to [create an application
ID](https://developer.apple.com/account/ios/identifier/bundle/create).
For an App ID to use Push Notifications, it must be created as an
**Explicit App ID**. Please be sure that under "**App Services**" you
select **Push Notifications**. Once these values are set, click on
\[Continue\].

Confirm that the app settings are as you desire and click \[Register\],
or click \[Back\] and correct them. **Push Notifications** should appear
as "Configurable".

## Create the Certificate

Then [Create a new
certificate](https://developer.apple.com/account/ios/certificate/create).
Select "Apple Push Notification service SSL" for either Development or
Production, depending on intended usage of the certificate.
"Development", in this case, means a certificate that will not be used
by an application released for general public use, but instead only for
personal or team development. This is also known as a "Sandbox"
application and will require setting the "use_sandbox" flag. Once the
preferred option is selected, click \[Continue\].

Select the App ID that matches the Application that will use Push
Notifications. Several Application IDs may be present, be sure to match
the correct App ID. This will be the App ID which will act as the
recipient bridge for Push Notifications. Select \[Continue\].

Follow the on-screen instructions to generate a **CSR file**, click
\[Continue\], and upload the CSR.

Download the newly created
*iOSTeam_Provisioning_Profile\_.mobileprovision* keyset, and import it
into your **KeyChain Access** app.

## Exporting the .p12 key set

In **KeyChain Access**, for the **login** keychain, in the
**Certificates** category, you should find an **Apple Push Services:
\*your AppID**\* certificate. Right click on this certificate and select
*Export "Apple Push Services:"...*. Provide the file with a reasonably
unique name, such as `Push_Production_APNS_Keys.p12`, so that you can
find it easily later. You may wish to secure these keys with a password.

## Converting .p12 to PEM

You will need to convert the .p12 file to PEM format. *openssl* can
perform these steps for you. A simple script you could use might be:

``` bash
#!/bin/bash
echo Converting $1 to PEM
openssl pkcs12 -in $1 -out $1_cert.pem -clcerts -nokeys
openssl pkcs12 -in $1 -out $1_key.pem -nocerts -nodes
```

This will divide the p12 key into two components that can be read by the
autopush application.

## Sending the APNS message

The APNS post message contains JSON formatted data similar to the
following:

``` json
{
    "aps": {
        "content-available": 1
    },
    "key": "value",
    ...
}
```

*aps* is reserved as a sub-dictionary. All other *key*: *value* slots
are open.

In addition, you must specify the following headers:

  - *apns-id*: A lowercase, dash formatted UUID for this message.
  - *apns-priority*: Either **10** for Immediate delivery or **5** for
    delayable delivery.
  - *apns-topic*: The bundle ID for the recipient application. This must
    match the bundle ID of the AppID used to create the *"Apple Push
    Services:..."* certificate. It usually has the format of `com.example.ApplicationName`.
  - *apns-expiration*: The timestamp for when this message should expire
    in UTC based seconds. A zero ("0") means immediate expiration.

## Handling APNS responses

APNS returns a status code and an optional JSON block describing the
error. A list of [these responses are provided in the APNS
documentation](https://developer.apple.com/library/content/documentation/NetworkingInternet/Conceptual/RemoteNotificationsPG/CommunicatingwithAPNs.html)
( _**Note**_, Apple may change the document locaiton without warning. you may
be able to search using
[DeviceTokenNotForTopic](https://developer.apple.com/search/?q=DeviceTokenNotForTopic&type=Guides)
or similar error messages.)
