//! Various small utilities accumulated over time for the WebPush server
use std::time::Duration;

use futures::future::{Either, Future, FutureExt, IntoFuture, TryFuture, TryFutureExt};
use tokio::runtime::Handle;
use tokio::time;

use crate::errors::*;

pub mod timing;

pub use self::timing::{ms_since_epoch, sec_since_epoch, us_since_epoch};
