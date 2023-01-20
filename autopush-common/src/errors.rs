//! Error handling for Rust
//!
//! This module defines various utilities for handling errors in the Rust
//! thread. This uses the `error-chain` crate to ergonomically define errors,
//! enable them for usage with `?`, and otherwise give us some nice utilities.
//! It's expected that this module is always glob imported:
//!
//! ```ignore
//!     use errors::*;
//! ```
//!
//! And functions in general should then return `Result<()>`. You can add extra
//! error context via `chain_err`:
//!
//! ```ignore
//!     let e = some_function_returning_a_result().chain_err(|| {
//!         "some extra context here to make a nicer error"
//!     })?;
//! ```
//!
//! And you can also use the `MyFuture` type alias for "nice" uses of futures
//!
//! ```ignore
//!     fn add(a: i32) -> MyFuture<u32> {
//!         // ..
//!     }
//! ```
//!
//! You can find some more documentation about this in the `error-chain` crate
//! online.
use std::any::Any;
//use std::error;
use std::io;
use std::num;

use futures::Future;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Ws(#[from] tungstenite::Error),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Httparse(#[from] httparse::Error),
    #[error(transparent)]
    MetricError(#[from] cadence::MetricError),
    #[error(transparent)]
    UuidError(#[from] uuid::Error),
    #[error(transparent)]
    ParseIntError(#[from] num::ParseIntError),
    #[error(transparent)]
    ParseUrlError(#[from] url::ParseError),
    #[error(transparent)]
    ConfigError(#[from] config::ConfigError),

    #[error("thread panicked")]
    Thread(Box<dyn Any + Send>),
    #[error("websocket pong timeout")]
    PongTimeout,
    #[error("repeat uaid disconnect")]
    RepeatUaidDisconnect,
    #[error("invalid state transition, from: {0}, to: {1}")]
    InvalidStateTransition(String, String),
    #[error("invalid json: {0}")]
    InvalidClientMessage(String),
    #[error("server error fetching messages")]
    MessageFetch,
    #[error("unable to send to client")]
    SendError,
    #[error("client sent too many pings")]
    ExcessivePing,

    #[error("Broadcast Error: {0}")]
    BroadcastError(String),
    #[error("Payload Error: {0}")]
    PayloadError(String),
    #[error("General Error: {0}")]
    GeneralError(String),
    #[error("Database Error: {0}")]
    DatabaseError(String),
}

pub type Result<T> = std::result::Result<T, Error>;

pub type MyFuture<T> = Box<dyn Future<Item = T, Error = Error>>;
