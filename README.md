[![Build](https://travis-ci.org/mozilla-services/autopush-rs.svg?branch=master)](https://travis-ci.org/mozilla-services/autopush-rs)
[![License: MPL 2.0](https://img.shields.io/badge/License-MPL%202.0-brightgreen.svg)](https://opensource.org/licenses/MPL-2.0)

# Autopush-rs

Mozilla Push server built with [Rust](https://rust-lang.org).

This is the fourth generation of the Mozilla Web Push server. This is
a "work in progress", that currently supports websocket connections
and support for
[Megaphone](https://github.com/mozilla-services/megaphone) broadcast.

Please consult the [autopush
documentation](http://autopush.readthedocs.io/en/latest/index.html)
for information about how this server works, as well as any [error
messages](http://autopush.readthedocs.io/en/latest/http.html#error-codes)
you may see when sending push messages to our server.

MDN has information about [how to use
WebPush](https://developer.mozilla.org/en-US/docs/Web/API/Push_API)

***Note*** while `rust-doc` style comments are used prolifically
through the source, only public structures are rendered automatically.
For those curious about the inner workings, You may wish to read the
code files directly.

