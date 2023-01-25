#![recursion_limit = "1024"]
#![allow(clippy::result_large_err)] // add this until refactor of `crate::error::ApcError result too large`

#[macro_use]
extern crate slog;
#[macro_use]
extern crate slog_scope;

#[macro_use]
pub mod db;
pub mod endpoint;
pub mod errors;
pub mod logging;
pub mod notification;
#[macro_use]
pub mod util;
