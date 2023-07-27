[![License: MPL 2.0][mpl-svg]][mpl]
[![Build Status][circleci-badge]][circleci]
[![docs][docs-badge]][docs]
[![autoconnect API][autoconnect-API-docs-badge]][autoconnect-API-docs]
[![autoendpoint API][autoendpoint-API-docs-badge]][autoendpoint-API-docs]
[![Connect to Matrix via the Riot webapp][matrix-badge]][matrix]

# Autopush-rs

Mozilla Push server built with [Rust](https://rust-lang.org).

This is the fourth generation of the Mozilla Web Push server. It currently supports websocket connections
and support for
[Megaphone](https://github.com/mozilla-services/megaphone) broadcast.

Please consult the [autopush documentation][docs] for information
about how this server works, as well as any [error messages] you may
see when sending push messages to our server.

MDN has information about [how to use
WebPush](https://developer.mozilla.org/en-US/docs/Web/API/Push_API).

***Note*** while `rust-doc` style comments are used prolifically
through the source, only public structures are rendered automatically.
For those curious about the inner workings, You may wish to read the
code files directly.

## Debugging on Mobile

Mobile devices can specify the Push Server URL via the "[secret settings](https://github.com/mozilla-mobile/fenix/wiki/%22Secret-settings%22-debug-menu-instructions)".

_Do not use the mobile `about:config` menu settings. These are not read or used by the mobile browser._

The secret settings can be activatedby following [these instructions](https://github.com/mozilla-mobile/fenix/wiki/%22Secret-settings%22-debug-menu-instructions). Once the secret menu is active, select
**Sync Debug** from the the mobile **Settings** menu, and specify the **Custom Push server** URL.

**NOTE:** the default Push server url is `wss://push.services.mozilla.com/`

[mpl-svg]: https://img.shields.io/badge/License-MPL%202.0-blue.svg
[mpl]: https://opensource.org/licenses/MPL-2.0
[circleci-badge]: https://circleci.com/gh/mozilla-services/autopush-rs.svg?style=shield
[circleci]: https://circleci.com/gh/mozilla-services/autopush-rs
[autoconnect-API-docs-badge]: https://img.shields.io/badge/autoconnect%20API%20-%20docs%20-%20light%20green
[autoconnect-API-docs]: https://mozilla-services.github.io/autopush-rs/api/autoconnect/
[autoendpoint-API-docs-badge]: https://img.shields.io/badge/autoendpoint%20API%20-%20docs%20-%20light%20green
[autoendpoint-API-docs]: https://mozilla-services.github.io/autopush-rs/api/autoendpoint/
[docs-badge]: https://github.com/mozilla-services/autopush-rs/actions/workflows/publish_docs.yml/badge.svg
[docs]: https://mozilla-services.github.io/autopush-rs/
[matrix-badge]: https://img.shields.io/badge/chat%20on%20[m]-%23push%3Amozilla.org-blue
[matrix]: https://chat.mozilla.org/#/room/#push:mozilla.org
[error messages]: https://mozilla-services.github.io/autopush-rs/errors.html
