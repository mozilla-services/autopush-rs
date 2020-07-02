//! Various small utilities accumulated over time for the WebPush server
use std::collections::HashMap;
use std::hash::Hash;
use std::time::Duration;

use futures::future::{Either, Future, IntoFuture};
use tokio_core::reactor::{Handle, Timeout};

use crate::errors::*;

mod send_all;
pub mod timing;

pub use self::send_all::MySendAll;
pub use self::timing::{ms_since_epoch, sec_since_epoch, us_since_epoch};

/// Convenience future to time out the resolution of `f` provided within the
/// duration provided.
///
/// If the `dur` is `None` then the returned future is equivalent to `f` (no
/// timeout) and otherwise the returned future will cancel `f` and resolve to an
/// error if the `dur` timeout elapses before `f` resolves.
pub fn timeout<F>(f: F, dur: Option<Duration>, handle: &Handle) -> MyFuture<F::Item>
where
    F: Future + 'static,
    F::Error: Into<Error>,
{
    let dur = match dur {
        Some(dur) => dur,
        None => return Box::new(f.map_err(|e| e.into())),
    };
    let timeout = Timeout::new(dur, handle).into_future().flatten();
    Box::new(f.select2(timeout).then(|res| match res {
        Ok(Either::A((item, _timeout))) => Ok(item),
        Err(Either::A((e, _timeout))) => Err(e.into()),
        Ok(Either::B(((), _item))) => Err("timed out".into()),
        Err(Either::B((e, _item))) => Err(e.into()),
    }))
}

pub trait InsertOpt<K: Eq + Hash, V> {
    /// Insert an item only if it exists
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>);
}

impl<K: Eq + Hash, V> InsertOpt<K, V> for HashMap<K, V> {
    fn insert_opt(&mut self, key: impl Into<K>, value: Option<impl Into<V>>) {
        if let Some(value) = value {
            self.insert(key.into(), value.into());
        }
    }
}
