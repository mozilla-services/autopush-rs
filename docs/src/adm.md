# Configuring the Amazon Device Messaging Bridge

<div style="color:red;border:4px solid red;padding:10px;margin 10px;">ADM is no longer supported by Autopush. This section is obsolete.
</div>

[ADM](https://developer.amazon.com/docs/adm/overview.html) requires
credentials that are provided on the [Amazon Developer
portal](https://developer.amazon.com/myapps.html) page. _**Note**_, this is
different than the *Amazon Web Services* page.

If you've not already done so, create a new App under the **Apps &
Services** tab. You will need to create an app so that you can associate
a Security Profile to it.

Device Messaging can be created by generating a new *Security Profile*
(located under the *Security Profiles* sub-tab. If specifying for
Android or Kindle, you will need to provide the Java Package name you've
used to identify the application (e.g. <span
class="title-ref">org.mozilla.services.admpushdemo</span>)

You will need to provide the MD5 Signature and SHA256 Signature for the
package's Certificate.

## Getting the Key Signatures

Amazon provides [some instructions \<https://developer.amazon
.com/docs/login-with-amazon/register-android.html#app-signatures-and-keys\>]()
for getting the signature values of the <span
class="title-ref">CERT.RSA</span> file. Be aware that android and ADM
are both moving targets and some information may no longer be correct.

I was able to use the `keytool` to fetch
out the SHA256 signature, but had to get the MD5 signature from inside
**Android Studio** by looking under the *Gradle* tab, then under the
Project (root)

``` text
> Task
  > android
   * signingReport
```

You do not need the SHA1: key provided from the signingReport output.

Once the fields have been provided an API Key will be generated. This is
a long JWT that must be stored in a file named <span
class="title-ref">api_key.txt</span> located in the <span
class="title-ref">/assets</span> directory. The file should only contain
the key. Extra white space, comments, or other data will cause the key
to fail to be read.

This file *MUST* be included with any client application that uses the
ADM bridge. Please note that the only way to test ADM messaging features
is to side load the application on a FireTV or Kindle device.

## Configuring the server

The server requires the *Client ID* and *Client Secret* from the ADM
Security Profile page. Since a given server may need to talk to
different applications using different profiles, the server can be
configured to use one of several profiles.

The `autopush_endpoint.ini` file may
contain the `adm_creds` option. This is a
JSON structure similar to the APNS configuration. The configuration can
specify one or more "profiles". Each profile contains a "client_id" and
"client_secret".

For example, let's say that we want to have a "dev" (for developers) and
a "stage" (for testing). We could specify the profiles as:

``` json
{
  "dev": {
     "client_id": "amzn1.application.0e7299...",
     "client_secret": "559dac53757a571d2fee78e5fcb2..."
   },
  "stage": {
     "client_id": "amzn1.application.0e7300...",
     "client_secret": "589dcc53957a971d2fee78e5fee4..."
   },
}
```

For the configuration, we'd collapse this to one line, e.g.

``` text
adm_creds={"dev":{"client_id":"amzn1.application.0e7299...","client_secret":"559dac53757a571d2fee78e5fcb2..."},"stage":{"client_id":"amzn1.application.0e7300...","client_secret": "589dcc53957a971d2fee78e5fee4..."},}
```

Much like other systems, a sender invokes the profile by using it in the
Registration URL. e.g. to register a new endpoint using the <span
class="title-ref">dev</span> profile:

> <span
> class="title-ref">https://push.service.mozilla.org/v1/adm/dev/registration/</span>
