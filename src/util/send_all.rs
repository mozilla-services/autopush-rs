use futures::stream::Fuse;
use futures::{Async, AsyncSink, Future, Poll, Sink, Stream};

// This is a copy of `Future::forward`, except that it doesn't close the sink
// when it's finished.
pub struct MySendAll<T: Stream, U> {
    sink: Option<U>,
    stream: Option<Fuse<T>>,
    buffered: Option<T::Item>,
}

impl<T, U> MySendAll<T, U>
where
    U: Sink<SinkItem = T::Item>,
    T: Stream,
    T::Error: From<U::SinkError>,
{
    #[allow(unused)]
    pub fn new(t: T, u: U) -> MySendAll<T, U> {
        MySendAll {
            sink: Some(u),
            stream: Some(t.fuse()),
            buffered: None,
        }
    }

    fn sink_mut(&mut self) -> &mut U {
        self.sink
            .as_mut()
            .take()
            .expect("Attempted to poll MySendAll after completion")
    }

    fn stream_mut(&mut self) -> &mut Fuse<T> {
        self.stream
            .as_mut()
            .take()
            .expect("Attempted to poll MySendAll after completion")
    }

    fn take_result(&mut self) -> (T, U) {
        let sink = self.sink
            .take()
            .expect("Attempted to poll MySendAll after completion");
        let fuse = self.stream
            .take()
            .expect("Attempted to poll MySendAll after completion");
        (fuse.into_inner(), sink)
    }

    fn try_start_send(&mut self, item: T::Item) -> Poll<(), U::SinkError> {
        debug_assert!(self.buffered.is_none());
        if let AsyncSink::NotReady(item) = try!(self.sink_mut().start_send(item)) {
            self.buffered = Some(item);
            return Ok(Async::NotReady);
        }
        Ok(Async::Ready(()))
    }
}

impl<T, U> Future for MySendAll<T, U>
where
    U: Sink<SinkItem = T::Item>,
    T: Stream,
    T::Error: From<U::SinkError>,
{
    type Item = (T, U);
    type Error = T::Error;

    fn poll(&mut self) -> Poll<(T, U), T::Error> {
        // If we've got an item buffered already, we need to write it to the
        // sink before we can do anything else
        if let Some(item) = self.buffered.take() {
            try_ready!(self.try_start_send(item))
        }

        loop {
            match try!(self.stream_mut().poll()) {
                Async::Ready(Some(item)) => try_ready!(self.try_start_send(item)),
                Async::Ready(None) => {
                    try_ready!(self.sink_mut().poll_complete());
                    return Ok(Async::Ready(self.take_result()));
                }
                Async::NotReady => {
                    try_ready!(self.sink_mut().poll_complete());
                    return Ok(Async::NotReady);
                }
            }
        }
    }
}
