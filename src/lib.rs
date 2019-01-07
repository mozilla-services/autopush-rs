//! WIP: Implementation of Web Push ("autopush" as well) in Rust
//!
//! This crate currently provides the connection node functionality for a Web
//! Push server, and is a replacement for the
//! [`autopush`](https://github.com/mozilla-services/autopush) server. The older
//! python handler still manages the HTTP endpoint calls, since those require
//! less overhead to process. Eventually, those functions will be moved to
//! rust as well.
//!
//! # High level overview
//!
//! The entire server is written in an asynchronous fashion using the `futures`
//! crate in Rust. This basically just means that everything is exposed as a
//! future (similar to the concept in other languages) and that's how bits and
//! pieces are chained together.
//!
//! Each connected client maintains a state machine of where it is in the
//! webpush protocol (see `states.dot` at the root of this repository). Note
//! that not all states are implemented yet, this is a work in progress! All I/O
//! is managed by Rust and various state transitions are managed by Rust as
//! well.
//!
//! # Module index
//!
//! There's a number of modules that currently make up the Rust implementation,
//! and one-line summaries of these are:
//!
//! * `queue` - a MPMC queue which is used to send messages to Python and Python
//!   uses to delegate work to worker threads.
//! * `server` - the main bindings for the WebPush server, where the tokio
//!   `Core` is created and managed inside of the Rust thread.
//! * `client` - this is where the state machine for each connected client is
//!   located, managing connections over time and sending out notifications as
//!   they arrive.
//! * `protocol` - a definition of the Web Push protocol messages which are send
//!   over websockets.
//! * `call` - definitions of various calls that can be made into Python, each
//!   of which returning a future of the response.
//!
//! Other modules tend to be miscellaneous implementation details and likely
//! aren't as relevant to the Web Push implementation.
//!
//! Otherwise be sure to check out each module for more documentation!
// `error_chain!` can recurse deeply
#![recursion_limit = "1024"]
use base64;

use cadence;


use config;


#[macro_use]
extern crate futures;

use hex;
use httparse;
use hyper;
#[macro_use]
extern crate lazy_static;

#[macro_use]
extern crate matches;
use mozsvc_common;



use reqwest;



#[macro_use]
extern crate sentry;

#[macro_use]
extern crate serde_derive;
use serde_dynamodb;
#[macro_use]
extern crate serde_json;
#[macro_use]
extern crate slog;
use slog_async;

#[macro_use]
extern crate slog_scope;
use slog_stdlog;
use slog_term;
#[macro_use]
extern crate state_machine_future;
use time;

use tokio_io;



use tungstenite;
use uuid;


#[macro_use]
extern crate error_chain;

#[macro_use]
mod db;
mod client;
pub mod errors;
mod http;
mod logging;
mod protocol;
pub mod server;
pub mod settings;
#[macro_use]
mod util;
