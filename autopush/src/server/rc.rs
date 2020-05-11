use std::cell::{RefCell, RefMut};
use std::rc::Rc;
use std::pin::Pin;

use futures::task::{Context, Poll};
use futures::sink::Sink;
use futures::stream::Stream;
use crate::server::protocol::{ServerMessage};

/// Helper object to turn `Rc<RefCell<T>>` into a `Stream` and `Sink`
///
/// This is basically just a helper to allow multiple "owning" references to a
/// `T` which is both a `Stream` and a `Sink`. Similar to `Stream::split` in the
/// futures crate, but doesn't actually split it (and allows internal access).
pub struct RcObject<T>(Rc<RefCell<T>>);

impl<T> RcObject<T> {
    pub fn new(t: T) -> RcObject<T> {
        RcObject(Rc::new(RefCell::new(t)))
    }

    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        self.0.borrow_mut()
    }
}

impl<T: Stream> Stream for RcObject<T> {
    type Item = T::Item;

    fn poll_next(&mut self) -> Poll<Option<T::Item>> {
        self.0.borrow_mut().poll_next()
    }
}

impl<T: Sink<ServerMessage>> Sink<ServerMessage> for RcObject<T> {
    type Error = T::Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Result<(), Self::Error>> {
        self.0.borrow_mut().poll_ready(cx)
    }

    fn start_send(self: Pin<&mut Self>, msg: ServerMessage) -> Result<(), Self::Error> {
        self.0.borrow_mut().start_send(msg)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Result<(), Self::Error>> {
        self.0.borrow_mut().poll_flush(cx)
    }

    fn poll_close(
        self: Pin<&mut Self>,
        cx: &mut Context
    ) -> Poll<Result<(), Self::Error>> {
        self.0.borrow_mut().poll_close(cx)
    }
}

impl<T> Clone for RcObject<T> {
    fn clone(&self) -> RcObject<T> {
        RcObject(self.0.clone())
    }
}
