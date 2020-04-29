//! Various small utilities accumulated over time for the WebPush server
use std::time::Duration;

use futures::future::{Either, Future, IntoFuture};
use tokio::runtime::Handle;
use tokio::time::Timeout;

use crate::errors::*;

pub mod timing;

pub use self::timing::{ms_since_epoch, sec_since_epoch, us_since_epoch};

/// Convenience future to time out the resolution of `f` provided within the
/// duration provided.
///
/// If the `dur` is `None` then the returned future is equivalent to `f` (no
/// timeout) and otherwise the returned future will cancel `f` and resolve to an
/// error if the `dur` timeout elapses before `f` resolves.
pub fn timeout<F>(f: F, dur: Option<Duration>, handle: &Handle) -> MyFuture<F::Output>
where
    F: Future + 'static,
    F: Into<Error>,
{
    let dur = match dur {
        Some(dur) => dur,
        None => return Box::new(f.map_err(|e| e.into())),
    };
    let timeout = Timeout::new(dur, handle).into_future().flatten();
    Box::new(f.select2(timeout).then(|res| match res {
        Ok(Either::Left((item, _timeout))) => Ok(item),
        Err(Either::Left((e, _timeout))) => Err(e.into()),
        Ok(Either::Right(((), _item))) => Err("timed out".into()),
        Err(Either::Right((e, _item))) => Err(e.into()),
    }))
}
