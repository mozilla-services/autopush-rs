//! wrap the old webpush socket code.
//!
//! [ed] This is mostly for ping handling for the broadcast code and can
//! probably be dropped. The only bit that is needed is how the
//! ping body consists of a json! hashmap. See `send_ws_ping`.

use std::env;
use std::task::Poll;

use futures::{Future, Sink, Stream};
//use futures_util::compat::Compat01As03Sink;
// use futures_01::StartSend;
use hyper::StatusCode;
use serde_json::{self, json};
use tungstenite::{self, Message};

use autopush_common::errors::{ApiError, ApiErrorKind};

use crate::megaphone::Broadcast;
use crate::server::protocol::{ClientMessage, ServerMessage};
use crate::server::webpush_io::WebpushIo;

// Wrapper struct to take a Sink/Stream of `Message` to a Sink/Stream of
// `ClientMessage` and `ServerMessage`.
pub struct WebpushSocket<T> {
    inner: T,
    ws_pong_received: bool,
    ws_ping: bool,
    ws_pong_timeout: bool,
    broadcast_delta: Option<Vec<Broadcast>>,
}

impl<T> WebpushSocket<T> {
    fn new(t: T) -> WebpushSocket<T> {
        WebpushSocket {
            inner: t,
            ws_pong_received: false,
            ws_ping: false,
            ws_pong_timeout: false,
            broadcast_delta: None,
        }
    }

    fn send_ws_ping(&mut self) -> Poll<(), ApiError>
    where
        T: Sink<SinkItem = Message>,
    {
        if self.ws_ping {
            let msg = if let Some(broadcasts) = self.broadcast_delta.clone() {
                debug!("sending a broadcast delta");
                let server_msg = ServerMessage::Broadcast {
                    broadcasts: Broadcast::vec_into_hashmap(broadcasts),
                };
                let s = server_msg.to_json().map_err(|| "failed to serialize")?;
                Message::Text(s)
            } else {
                debug!("sending a ws ping");
                Message::Ping(Vec::new())
            };
            match self.inner.start_send(msg)? {
                AsyncSink::Ready => {
                    debug!("ws ping sent");
                    self.ws_ping = false;
                }
                AsyncSink::NotReady(_) => {
                    debug!("ws ping not ready to be sent");
                    return Ok(Async::NotReady);
                }
            }
        }
        Ok(Async::Ready(()))
    }
}

impl<T> Stream for WebpushSocket<T>
where
    T: Stream<Item = Message>,
{
    type Item = ClientMessage;

    fn poll_next(&mut self) -> Poll<Option<ClientMessage>> {
        loop {
            let msg = match self.inner.poll()? {
                Async::Ready(Some(msg)) => msg,
                Async::Ready(None) => return Ok(None.into()),
                Async::NotReady => {
                    // If we don't have any more messages and our pong timeout
                    // elapsed (set above) then this is where we start
                    // triggering errors.
                    if self.ws_pong_timeout {
                        return Err(ApiErrorKind::PongTimeout.into());
                    }
                    return Ok(Async::NotReady);
                }
            };
            match msg {
                Message::Text(ref s) => {
                    trace!("text message {}", s);
                    let msg = s
                        .parse()
                        .map_err(|_| ApiErrorKind::InvalidClientMessage(s.to_owned()))?;
                    return Ok(Some(msg).into());
                }

                Message::Binary(_) => {
                    return Err(
                        ApiErrorKind::InvalidClientMessage("binary content".to_string()).into(),
                    );
                }

                // sending a pong is already managed by lower layers, just go to
                // the next message
                Message::Ping(_) => {}

                // Wake up ourselves to ensure the above ping logic eventually
                // sees this pong.
                Message::Pong(_) => {
                    self.ws_pong_received = true;
                    self.ws_pong_timeout = false;
                    std::thread::current().notify();
                }

                Message::Close(_) => return Err(tungstenite::Error::ConnectionClosed.into()),
                Message::Frame(_) => {
                    return Err(ApiErrorKind::InvalidClientMessage(
                        "frame content".to_string(),
                    ))
                    .into()
                }
            }
        }
    }
}

impl<T> Sink<ServerMessage> for WebpushSocket<T>
where
    T: Sink<Message>,
{
    fn start_send(&mut self, msg: ServerMessage) -> StartSend<ServerMessage, ApiError> {
        if self.send_ws_ping()?.is_not_ready() {
            return Ok(AsyncSink::NotReady(msg));
        }
        let s = msg.to_json()?;
        match self.inner.start_send(Message::Text(s))? {
            AsyncSink::Ready => Ok(AsyncSink::Ready),
            AsyncSink::NotReady(_) => Ok(AsyncSink::NotReady(msg)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Error> {
        try_ready!(self.send_ws_ping());
        Ok(self.inner.poll_complete()?)
    }

    fn close(&mut self) -> Poll<(), Error> {
        try_ready!(self.poll_complete());
        Ok(self.inner.close()?)
    }
}

fn write_status(socket: WebpushIo) -> MyFuture<()> {
    write_json(
        socket,
        StatusCode::OK,
        json!({
                "status": "OK",
                "version": env!("CARGO_PKG_VERSION"),
        }),
    )
}
