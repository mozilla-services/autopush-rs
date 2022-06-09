/// Async based client that is less dependent on a state machine trait
///
/// The previous state machine required manipulating inner workings of
/// `future 0.1` to work correctly. Current, simpler state machine
/// frameworks like `rust_fsm` are too simplistic for what we need,
/// offering  only separate state transition and output direction.
///
/// States are fairly simple in autopush, you can be:
/// Pending Authorization or In A Command Loop. You only ever transition
/// from Pending Authorization to Command Loop, since Errors disconnect
/// the client with an error message.
///
/// The Pending Authorization path is:
/// Client connection => wait TIMEOUT for initial `hello` message type
/// Once the `hello` has been validated and processed, we can drop
/// immediately into the Command loop.
///
/// The Command Loop consists of an infinite loop that awaits the socket
/// times out after

/// TODO: actually build out all of that, based on actix-websocket

use std::cell::RefCell;
use std::mem;
use std::rc::Rc;
use std::time::Duration;

use actix::{Actor, StreamHandler};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use cadence::{prelude::*, StatsdClient};
use error_chain::ChainedError;
use futures::future::Either;
//use futures::sync::mpsc;
use futures::channel::{mpsc, oneshot::Receiver};
//use futures::AsyncSink;
use futures::future;
use futures::{task::Poll, Future, Sink, Stream};
use reqwest::Client as ReqClient;
// use reqwest::r#async::Client as AsyncClient;
use rusoto_dynamodb::UpdateItemOutput;
// use state_machine_future::{transition, RentToOwn, StateMachineFuture};
use rust_fsm::{StateMachine, StateMachineImpl};
use serde_json::Value;
use tokio_core::reactor::Timeout;
use uuid::Uuid;

use autopush_common::db::{CheckStorageResponse, HelloResponse, RegisterResponse, UserRecord};
use autopush_common::error::ApiResult;
use autopush_common::endpoint::make_endpoint;
use autopush_common::notification::Notification;
use autopush_common::util::{ms_since_epoch, sec_since_epoch};

use crate::megaphone::{Broadcast, BroadcastSubs};
use crate::server::aserver::Server;
use crate::server::protocol::{ClientMessage, ServerMessage, ServerNotification};
use crate::user_agent::parse_user_agent;


// Created and handed to the AutopushServer
pub struct RegisteredClient {
    pub uaid: Uuid,
    pub uid: Uuid,
    pub tx: mpsc::UnboundedSender<ServerNotification>,
}

struct Client;

impl Actor for Client {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for Client {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                ctx.pong(&msg)
            },
            Ok(ws::Message::Text(msg)) => {
                async move {
                    let message:Value = serde_json::from_slice(msg)?;
                    self
                }
                // process message
            },
            Ok(ws::Message::Binary(msg)) => {
                // Not Implemented
            },
            _ => {()},
        }
    }
}

impl Client {
    fn process_message(msg: Value) -> ApiResult<()> {

    }
}

async fn handle_ws(req: HttpRequest, stream: web::Payload) -> Result<HttpResponse, Error> {
    let resp = ws::start(Client{}, &req, stream);
    resp
}

/*
pub struct Client<T>
where
    T: Stream<Item = ClientMessage> + Sink<ServerMessage> + 'static,
{
    // state_machine: UnAuthClientStateFuture<T>,
    srv: Rc<Server>,
    broadcast_subs: Rc<RefCell<BroadcastSubs>>,
    tx: mpsc::UnboundedSender<ServerNotification>,
}

impl<T> Client<T>
where
    T: Stream<Item = ClientMessage> + Sink<ServerMessage> + 'static,
{
    /// Spins up a new client communicating over the websocket `ws` specified.
    ///
    /// The `ws` specified already has ping/pong parts of the websocket
    /// protocol managed elsewhere, and this struct is only expected to deal
    /// with webpush-specific messages.
    ///
    /// The `srv` argument is the server that this client is attached to and
    /// the various state behind the server. This provides transitive access to
    /// various configuration options of the server as well as the ability to
    /// call back into Python.
    pub fn new(ws: T, srv: &Rc<Server>, mut uarx: Receiver<String>) -> Client<T> {
        let srv = srv.clone();
        let timeout = Timeout::new(srv.opts.open_handshake_timeout.unwrap(), &srv.handle).unwrap();
        let (tx, rx) = mpsc::unbounded();

        // Pull out the user-agent, which we should have by now
        let uastr = match uarx.poll() {
            Ok(Async::Ready(ua)) => ua,
            Ok(Async::NotReady) => {
                error!("Failed to parse the user-agent");
                String::from("")
            }
            Err(_) => {
                error!("Failed to receive a value");
                String::from("")
            }
        };

        let broadcast_subs = Rc::new(RefCell::new(Default::default()));
        /*
        let sm = UnAuthClientState::start(
            UnAuthClientData {
                srv: srv.clone(),
                ws,
                user_agent: uastr,
                broadcast_subs: broadcast_subs.clone(),
            },
            timeout,
            tx.clone(),
            rx,
        );
        // */

        Self {
            // state_machine: sm,
            srv,
            broadcast_subs,
            tx,
        }
    }

    pub fn broadcast_delta(&mut self) -> Option<Vec<Broadcast>> {
        let mut broadcast_subs = self.broadcast_subs.borrow_mut();
        self.srv.broadcast_delta(&mut broadcast_subs)
    }

    pub fn shutdown(&mut self) {
        let _result = self.tx.unbounded_send(ServerNotification::Disconnect);
    }
}
*/
