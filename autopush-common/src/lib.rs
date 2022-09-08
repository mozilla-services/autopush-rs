#![recursion_limit = "1024"]

/*
#[macro_use]
extern crate error_chain;
// */
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
