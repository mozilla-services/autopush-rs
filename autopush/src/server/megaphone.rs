//! [ed]
//! some of the megaphone functions and elements extricated out
//! of the server code. There are still a few dependent functions
//! inside of the old_server::Server that should also be pulled
//! into here.

// TODO: update this to be async favorable. It currently HEAVILY uses
// old futures poll()/ready. git 
use std::io;
use std::rc::Rc;
use std::task::Poll;
use std::time::{Duration, Instant};

use futures::{channel::oneshot, Future};
// use futures_util::compat::Compat01As03Sink
use sentry::{self, capture_message};
use tokio_core::reactor::Timeout;
use tokio_tungstenite::WebSocketStream;

// use autopush_common::db::postgres::PostgresStorage;
use autopush_common::errors::{ApiError, ApiErrorKind};

use crate::megaphone::{Broadcast, MegaphoneAPIResponse};
use crate::old_client::Client;
use crate::server::old_server::Server;
use crate::server::old_webpush_socket::WebpushSocket;
use crate::server::rc::RcObject;
use crate::server::webpush_io::WebpushIo;

enum MegaphoneState {
    Waiting,
    Requesting(MegaphoneAPIResponse),
}

struct MegaphoneUpdater {
    srv: Rc<Server>,
    api_url: String,
    api_token: String,
    state: MegaphoneState,
    timeout: Timeout,
    poll_interval: Duration,
    client: reqwest::Client,
}

impl MegaphoneUpdater {
    fn new(
        uri: &str,
        token: &str,
        poll_interval: Duration,
        srv: &Rc<Server>,
    ) -> io::Result<MegaphoneUpdater> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(1))
            .build()
            .expect("Unable to build reqwest client");
        Ok(MegaphoneUpdater {
            srv: srv.clone(),
            api_url: uri.to_string(),
            api_token: token.to_string(),
            state: MegaphoneState::Waiting,
            timeout: Timeout::new(poll_interval, &srv.handle)?,
            poll_interval,
            client,
        })
    }
}

impl Future for MegaphoneUpdater {
    type Output = ();

    fn poll(&mut self) -> Poll<()> {
        loop {
            let new_state = match self.state {
                MegaphoneState::Waiting => {
                    debug!("Sending megaphone API request");
                    let fut = self
                        .client
                        .get(&self.api_url)
                        .header("Authorization", self.api_token.clone())
                        .send()
                        .and_then(|response| response.error_for_status())
                        .and_then(|mut response| response.json())
                        .map_err(|_| "Unable to query/decode the API query".into());
                    MegaphoneState::Requesting(Box::new(fut))
                }
                MegaphoneState::Requesting(ref mut response) => {
                    let at = Instant::now() + self.poll_interval;
                    match response.poll() {
                        Ok(Async::Ready(MegaphoneAPIResponse { broadcasts })) => {
                            debug!("Fetched broadcasts: {:?}", broadcasts);
                            let mut broadcaster = self.srv.broadcaster.borrow_mut();
                            for srv in Broadcast::from_hashmap(broadcasts) {
                                broadcaster.add_broadcast(srv);
                            }
                        }
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(error) => {
                            error!("Failed to get response, queue again {:?}", error);
                            capture_message(
                                &format!("Failed to get response, queue again {:?}", error),
                                sentry::Level::Error,
                            );
                        }
                    };
                    self.timeout.reset(at);
                    MegaphoneState::Waiting
                }
            };
            self.state = new_state;
        }
    }
}

enum WaitingFor {
    SendPing,
    Pong,
    Close,
}

enum CloseState<T> {
    Exchange(T),
    Closing,
}

pub struct PingManager {
    socket: RcObject<WebpushSocket<WebSocketStream<WebpushIo>>>,
    timeout: Timeout,
    waiting: WaitingFor,
    srv: Rc<Server>,
    client: CloseState<Client<RcObject<WebpushSocket<WebSocketStream<WebpushIo>>>>>,
}

impl PingManager {
    fn new(
        srv: &Rc<Server>,
        socket: WebSocketStream<WebpushIo>,
        uarx: oneshot::Receiver<String>,
    ) -> io::Result<PingManager> {
        // The `socket` is itself a sink and a stream, and we've also got a sink
        // (`tx`) and a stream (`rx`) to send messages. Half of our job will be
        // doing all this proxying: reading messages from `socket` and sending
        // them to `tx` while also reading messages from `rx` and sending them
        // on `socket`.
        //
        // Our other job will be to manage the websocket protocol pings going
        // out and coming back. The `opts` provided indicate how often we send
        // pings and how long we'll wait for the ping to come back before we
        // time it out.
        //
        // To make these tasks easier we start out by throwing the `socket` into
        // an `Rc` object. This'll allow us to share it between the ping/pong
        // management and message shuffling.
        let socket = RcObject::new(WebpushSocket::new(socket));
        Ok(PingManager {
            timeout: Timeout::new(srv.opts.auto_ping_interval, &srv.handle)?,
            waiting: WaitingFor::SendPing,
            socket: socket.clone(),
            client: CloseState::Exchange(Client::new(socket, srv, uarx)),
            srv: srv.clone(),
        })
    }
}

impl Future for PingManager {
    type Output = ();

    fn poll(&mut self) -> Poll<()> {
        let mut socket = self.socket.borrow_mut();
        loop {
            if socket.ws_ping {
                // Don't check if we already have a delta to broadcast
                if socket.broadcast_delta.is_none() {
                    // Determine if we can do a broadcast check, we need a connected webpush client
                    if let CloseState::Exchange(ref mut client) = self.client {
                        if let Some(delta) = client.broadcast_delta() {
                            socket.broadcast_delta = Some(delta);
                        }
                    }
                }

                if socket.send_ws_ping()?.is_ready() {
                    // If we just sent a broadcast, reset the ping interval and clear the delta
                    if socket.broadcast_delta.is_some() {
                        let at = Instant::now() + self.srv.opts.auto_ping_interval;
                        self.timeout.reset(at);
                        socket.broadcast_delta = None;
                        self.waiting = WaitingFor::SendPing
                    } else {
                        let at = Instant::now() + self.srv.opts.auto_ping_timeout;
                        self.timeout.reset(at);
                        self.waiting = WaitingFor::Pong
                    }
                } else {
                    break;
                }
            }
            debug_assert!(!socket.ws_ping);
            match self.waiting {
                WaitingFor::SendPing => {
                    debug_assert!(!socket.ws_pong_timeout);
                    debug_assert!(!socket.ws_pong_received);
                    match self.timeout.poll()? {
                        Async::Ready(()) => {
                            debug!("scheduling a ws ping to get sent");
                            socket.ws_ping = true;
                        }
                        Async::NotReady => break,
                    }
                }
                WaitingFor::Pong => {
                    if socket.ws_pong_received {
                        // If we received a pong, then switch us back to waiting
                        // to send out a ping
                        debug!("ws pong received, going back to sending a ping");
                        debug_assert!(!socket.ws_pong_timeout);
                        let at = Instant::now() + self.srv.opts.auto_ping_interval;
                        self.timeout.reset(at);
                        self.waiting = WaitingFor::SendPing;
                        socket.ws_pong_received = false;
                    } else if socket.ws_pong_timeout {
                        // If our socket is waiting to deliver a pong timeout,
                        // then no need to keep checking the timer and we can
                        // keep going
                        debug!("waiting for socket to see ws pong timed out");
                        break;
                    } else if self.timeout.poll()?.is_ready() {
                        // We may not actually be reading messages from the
                        // websocket right now, could have been waiting on
                        // something else. Instead of immediately returning an
                        // error here wait for the stream to return `NotReady`
                        // when looking for messages, as then we're extra sure
                        // that no pong was received after this timeout elapsed.
                        debug!("waited too long for a ws pong");
                        socket.ws_pong_timeout = true;
                    } else {
                        break;
                    }
                }
                WaitingFor::Close => {
                    debug_assert!(!socket.ws_pong_timeout);
                    if self.timeout.poll()?.is_ready() {
                        if let CloseState::Exchange(ref mut client) = self.client {
                            client.shutdown();
                        }
                        // So did the shutdown not work? We must call shutdown but no client here?
                        return Err("close handshake took too long".into());
                    }
                }
            }
        }

        // Be sure to always flush out any buffered messages/pings
        socket.poll_complete().map_err(|_| {
            ApiErrorKind::GeneralError("failed routine `poll_complete` call".to_owned())
        })?;
        drop(socket);

        // At this point looks our state of ping management A-OK, so try to
        // make progress on our client, and when done with that execute the
        // closing handshake.
        loop {
            match self.client {
                CloseState::Exchange(ref mut client) => client.poll(),
                CloseState::Closing => return self.socket.borrow_mut().close(),
            }

            self.client = CloseState::Closing;
            if let Some(dur) = self.srv.opts.close_handshake_timeout {
                let at = Instant::now() + dur;
                self.timeout.reset(at);
                self.waiting = WaitingFor::Close;
            }
        }
    }
}
