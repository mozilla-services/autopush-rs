# Autopush

Mozilla Push server and Push Endpoint utilizing Rust, Actix, and
a key/value data store.

This is the fourth generation of Push server built in Mozilla Services, and is built to support the the [W3C Push
spec](http://w3c.github.io/push-api/index.html).

For how to read and respond to **autopush error codes**, see
[Errors](errors.md).

For an overview of the Mozilla Push Service and where autopush fits in,
see the [Mozilla Push Service architecture
diagram](architecture.md#architecture).
This push service uses [websockets](https://developer.mozilla.org/en-US/docs/Web/API/WebSockets_API) to talk to Firefox, with a Push
endpoint that implements the [`WebPush`](https://www.rfc-editor.org/rfc/rfc8030.html) standard for its `http` API.

## Autopush APIs

For developers writing mobile applications in Mozilla, or web developers using Push on the web with Firefox.

<div class="toctree" maxdepth="2">

* [HTTP Endpoints for Notifications](http.md)
* [Push Service HTTP API](http.md#push-service-http-api)
  * [Lexicon](http.md#lexicon)
  * [Response](http.md#response)
  * [Error Codes](http.md#error-codes)
  * [Calls](http.md#calls)
* [Push Service Bridge HTTP Interface](http.md#push-service-bridge-http-interface)
  * [Lexicon](http.md#id3)
  * [Calls](http.md#id4)

</div>

## Running Autopush

If you just want to run autopush, for testing Push locally with Firefox,
or to deploy autopush to a production environment for Firefox.

<div class="toctree" maxdepth="2">

* [Architecture](architecture.md)
  * [Overview](architecture.md#overview)
  * [Cryptography](architecture.md#cryptography)
  * [Storage Tables](architecture.md#storage)
  * [Push Characteristics](architecture.md#push-characteristics)
  * [Push Reliability Tracking](reliability.md)
  * [Legacy table rotation](table_rotation.md)
* [Running Autopush](running.md)
  * [Overview](running.md#overview)
  * [Setup](running.md#setup)
  * [Configuration options](config_options.md)
  * [Start Autopush](running.md#start-autopush)
  * [Configuration](running.md#configuration)

</div>

## Developing Autopush

For developers wishing to work with the latest autopush source code,
it's recommended that you first familiarize yourself with
[running Autopush](running.md) before proceeding.

<div class="toctree" maxdepth="2">

* [Installing](install.md)
  * [System Requirements](install.md#requirements)
  * [Check-out the Autopush Repository](install.md#check-out)
  * [Build environment](install.md#build-env)
  * [Scripts](install.md#scripts)
  * [Building Documentation](install.md#building-documentation)
  * [Using a Local Storage Server](install.md#local-storage)
* [Testing](testing.md)
  * [Testing Configuration](testing.md#testing-configuration)
  * [Bigtable Google Cloud Emulation](bigtable-emulation.md)
  * [Running Tests](testing.md#running-tests)
  * [Firefox Testing](testing.md#firefox-testing)
* [Release Process](releasing.md)
  * [Versions](releasing.md#versions)
  * [Dev Releases](releasing.md#dev-releases)
  * [Stage/Production Releases](releasing.md#stage-production-releases)
* [Coding Style Guide](style.md)
  * [Exceptions](style.md#exceptions)

</div>

## Source Code

All source code is available on [github under
autopush](https://github.com/mozilla-services/autopush-rs).

<!-- TODO -->
* [autoconnect](api/autoconnect) - WebSocket server for desktop UAs
  * [autoconnect_common](api/autoconnect_common) - Common functions for autoconnect
  * [autoconnect_settings](api/autoconnect_settings) - Settings and configuration
  * [autoconnect_web](api/autoconnect_web) - HTTP functions
  * [autoconnect_ws](api/autoconnect_ws) - WebSocket functions
  * [autoconnect_ws_sm](api/autoconnect_ws_sm) - WebSocket state machine
* [autoendpoint](api/autoendpoint) - HTTP server for publication and mobile
* [autopush_common](api/autopush_common) - Common functions for autoconnect and autoendpoint

We are using [rust](https://rust-lang.org) for a number of optimizations
and speed improvements. These efforts are ongoing and may be subject to
change. Unfortunately, this also means that formal documentation is not
yet available. You are, of course, welcome to review the code located in
[`autopush-rs`](https://github.com/mozilla-services/autopush-rs).

## Changelog

[Changelog](https://github.com/mozilla-services/autopush-rs/blob/master/CHANGELOG.md)

## Bugs/Support

Bugs should be reported on the [autopush github issue
tracker](https://github.com/mozilla-services/autopush-rs/issues).

## autopush Endpoints

autopush is automatically deployed from master to a dev environment for
testing, a stage environment for tagged releases, and the production
environment used by Firefox/FirefoxOS.

### dev

* Websocket: <wss://autoconnect.dev.mozaws.net/>
* Endpoint: <https://updates-autopush.dev.mozaws.net/>

### stage

* Websocket: <wss://autoconnect.stage.mozaws.net/>
* Endpoint: <https://updates-autopush.stage.mozaws.net/>

### production

* Websocket: <wss://push.services.mozilla.com/>
* Endpoint: <https://updates.push.services.mozilla.com/>

## Reference

* [Glossary](glossary.md)
* [Why rust?](rust.md)

## License

`autopush` is offered under the Mozilla Public License 2.0.
