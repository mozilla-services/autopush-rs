//! Management of connected clients to a WebPush server
//!
//! This module is a pretty heavy work in progress. The intention is that
//! this'll house all the various state machine transitions and state management
//! of connected clients. Note that it's expected there'll be a lot of connected
//! clients, so this may appears relatively heavily optimized!
use std::cell::RefCell;
use std::mem;
use std::rc::Rc;

use cadence::prelude::*;
use futures::AsyncSink;
use futures::future::Either;
use futures::sync::mpsc;
use futures::sync::oneshot::Receiver;
use futures::{Async, Future, Poll, Sink, Stream};
use rusoto_dynamodb::UpdateItemOutput;
use state_machine_future::RentToOwn;
use tokio_core::reactor::Timeout;
use uuid::Uuid;
use woothee::parser::Parser;

use errors::*;
use protocol::{ClientMessage, Notification, ServerMessage, ServerNotification};
use server::Server;
use db::{CheckStorageResponse, HelloResponse, RegisterResponse};
use util::megaphone::{ClientServices, Service, ServiceClientInit};
use util::{ms_since_epoch, parse_user_agent, sec_since_epoch};

// Created and handed to the AutopushServer
pub struct RegisteredClient {
    pub uaid: Uuid,
    pub uid: Uuid,
    pub tx: mpsc::UnboundedSender<ServerNotification>,
}

pub struct Client<T>
    where
        T: Stream<Item=ClientMessage, Error=Error>
        + Sink<SinkItem=ServerMessage, SinkError=Error>
        + 'static,
{
    state_machine: UnAuthClientStateFuture<T>,
    srv: Rc<Server>,
    broadcast_services: Rc<RefCell<ClientServices>>,
    tx: mpsc::UnboundedSender<ServerNotification>,
}

impl<T> Client<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
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
    pub fn new(ws: T, srv: &Rc<Server>, mut uarx: Receiver<String>, host: String) -> Client<T> {
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

        let broadcast_services = Rc::new(RefCell::new(Default::default()));
        let sm = UnAuthClientState::start(
            UnAuthClientData {
                srv: srv.clone(),
                ws,
                user_agent: uastr,
                host,
                broadcast_services: broadcast_services.clone(),
            },
            timeout,
            tx.clone(),
            rx,
        );

        Self {
            state_machine: sm,
            srv: srv.clone(),
            broadcast_services,
            tx,
        }
    }

    pub fn broadcast_delta(&mut self) -> Option<Vec<Service>> {
        let mut broadcast_services = self.broadcast_services.borrow_mut();
        self.srv.broadcast_delta(&mut broadcast_services)
    }

    pub fn shutdown(&mut self) {
        let _result = self.tx.unbounded_send(ServerNotification::Disconnect);
    }
}

impl<T> Future for Client<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<(), Error> {
        self.state_machine.poll()
    }
}

// Websocket session statistics
#[derive(Clone, Default)]
struct SessionStatistics {
    // User data
    uaid: String,
    uaid_reset: bool,
    existing_uaid: bool,
    connection_type: String,
    host: String,

    // Usage data
    direct_acked: i32,
    direct_storage: i32,
    stored_retrieved: i32,
    stored_acked: i32,
    nacks: i32,
    unregisters: i32,
    registers: i32,
}

// Represent the state for a valid WebPush client that is authenticated
pub struct WebPushClient {
    uaid: Uuid,
    uid: Uuid,
    rx: mpsc::UnboundedReceiver<ServerNotification>,
    flags: ClientFlags,
    message_month: String,
    unacked_direct_notifs: Vec<Notification>,
    unacked_stored_notifs: Vec<Notification>,
    // Highest version from stored, retained for use with increment
    // when all the unacked storeds are ack'd
    unacked_stored_highest: Option<u64>,
    connected_at: u64,
    stats: SessionStatistics,
}

impl Default for WebPushClient {
    fn default() -> Self {
        let (_, rx) = mpsc::unbounded();
        Self {
            uaid: Default::default(),
            uid: Default::default(),
            rx,
            flags: Default::default(),
            message_month: Default::default(),
            unacked_direct_notifs: Default::default(),
            unacked_stored_notifs: Default::default(),
            unacked_stored_highest: Default::default(),
            connected_at: Default::default(),
            stats: Default::default(),
        }
    }
}

impl WebPushClient {
    fn unacked_messages(&self) -> bool {
        !self.unacked_stored_notifs.is_empty() || !self.unacked_direct_notifs.is_empty()
    }
}

#[derive(Default)]
pub struct ClientFlags {
    include_topic: bool,
    increment_storage: bool,
    check: bool,
    reset_uaid: bool,
    rotate_message_table: bool,
}

impl ClientFlags {
    fn new() -> Self {
        Self {
            include_topic: true,
            increment_storage: false,
            check: false,
            reset_uaid: false,
            rotate_message_table: false,
        }
    }
}

pub struct UnAuthClientData<T> {
    srv: Rc<Server>,
    ws: T,
    user_agent: String,
    host: String,
    broadcast_services: Rc<RefCell<ClientServices>>,
}

impl<T> UnAuthClientData<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    fn input_with_timeout(&mut self, timeout: &mut Timeout) -> Poll<ClientMessage, Error> {
        let item = match timeout.poll()? {
            Async::Ready(_) => return Err("Client timed out".into()),
            Async::NotReady => match self.ws.poll()? {
                Async::Ready(None) => return Err("Client dropped".into()),
                Async::Ready(Some(msg)) => Async::Ready(msg),
                Async::NotReady => Async::NotReady,
            },
        };
        Ok(item)
    }
}

pub struct AuthClientData<T> {
    srv: Rc<Server>,
    ws: T,
    webpush: Rc<RefCell<WebPushClient>>,
    broadcast_services: Rc<RefCell<ClientServices>>,
}

impl<T> AuthClientData<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    fn input_or_notif(&mut self) -> Poll<Either<ClientMessage, ServerNotification>, Error> {
        let mut webpush = self.webpush.borrow_mut();
        let item = match webpush.rx.poll() {
            Ok(Async::Ready(Some(notif))) => Either::B(notif),
            Ok(Async::Ready(None)) => return Err("Sending side dropped".into()),
            Ok(Async::NotReady) => match self.ws.poll()? {
                Async::Ready(None) => return Err("Client dropped".into()),
                Async::Ready(Some(msg)) => Either::A(msg),
                Async::NotReady => return Ok(Async::NotReady),
            },
            Err(_) => return Err("Unexpected error".into()),
        };
        Ok(Async::Ready(item))
    }
}

#[derive(StateMachineFuture)]
pub enum UnAuthClientState<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    #[state_machine_future(start, transitions(AwaitProcessHello))]
    AwaitHello {
        data: UnAuthClientData<T>,
        timeout: Timeout,
        tx: mpsc::UnboundedSender<ServerNotification>,
        rx: mpsc::UnboundedReceiver<ServerNotification>,
    },

    #[state_machine_future(transitions(AwaitSessionComplete))]
    AwaitProcessHello {
        response: MyFuture<HelloResponse>,
        data: UnAuthClientData<T>,
        interested_broadcasts: Vec<Service>,
        tx: mpsc::UnboundedSender<ServerNotification>,
        rx: mpsc::UnboundedReceiver<ServerNotification>,
    },

    #[state_machine_future(transitions(UnAuthDone))]
    AwaitSessionComplete {
        auth_state_machine: AuthClientStateFuture<T>,
        srv: Rc<Server>,
        user_agent: String,
        webpush: Rc<RefCell<WebPushClient>>,
    },

    #[state_machine_future(ready)]
    UnAuthDone(()),

    #[state_machine_future(error)]
    UnAuthClientStateError(Error),
}

impl<T> PollUnAuthClientState<T> for UnAuthClientState<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    fn poll_await_hello<'a>(
        hello: &'a mut RentToOwn<'a, AwaitHello<T>>,
    ) -> Poll<AfterAwaitHello<T>, Error> {
        trace!("State: AwaitHello");
        let (uaid, services) = {
            let AwaitHello {
                ref mut data,
                ref mut timeout,
                ..
            } = **hello;
            match try_ready!(data.input_with_timeout(timeout)) {
                ClientMessage::Hello {
                    uaid,
                    use_webpush: Some(true),
                    broadcasts,
                    ..
                } => (
                    uaid.and_then(|uaid| Uuid::parse_str(uaid.as_str()).ok()),
                    Service::from_hashmap(broadcasts.unwrap_or_default()),
                ),
                _ => return Err("Invalid message, must be hello".into()),
            }
        };

        let AwaitHello { data, tx, rx, .. } = hello.take();
        let connected_at = ms_since_epoch();
        let response = Box::new(data.srv.ddb.hello(
            &connected_at,
            uaid.as_ref(),
            &data.srv.opts.router_table_name,
            &data.srv.opts.router_url,
            &data.srv.opts.message_table_names,
            &data.srv.opts.current_message_month,
            &data.srv.metrics,
        ));
        transition!(AwaitProcessHello {
            response,
            data,
            interested_broadcasts: services,
            tx,
            rx,
        })
    }

    fn poll_await_process_hello<'a>(
        process_hello: &'a mut RentToOwn<'a, AwaitProcessHello<T>>,
    ) -> Poll<AfterAwaitProcessHello<T>, Error> {
        trace!("State: AwaitProcessHello");
        let (uaid, message_month, check_storage, reset_uaid, rotate_message_table, connected_at) = {
            match try_ready!(process_hello.response.poll()) {
                HelloResponse {
                    uaid: Some(uaid),
                    message_month,
                    check_storage,
                    reset_uaid,
                    rotate_message_table,
                    connected_at,
                } => (
                    uaid,
                    message_month,
                    check_storage,
                    reset_uaid,
                    rotate_message_table,
                    connected_at,
                ),
                HelloResponse { uaid: None, .. } => {
                    return Err("Already connected elsewhere".into())
                }
            }
        };

        let AwaitProcessHello {
            data,
            interested_broadcasts,
            tx,
            rx,
            ..
        } = process_hello.take();
        data.srv.metrics.incr("ua.command.hello").ok();

        let UnAuthClientData {
            srv,
            ws,
            user_agent,
            host,
            broadcast_services,
        } = data;

        // Setup the objects and such needed for a WebPushClient
        let mut flags = ClientFlags::new();
        flags.check = check_storage;
        flags.reset_uaid = reset_uaid;
        flags.rotate_message_table = rotate_message_table;
        let ServiceClientInit(client_services, broadcasts) =
            srv.broadcast_init(&interested_broadcasts);
        broadcast_services.replace(client_services);
        let uid = Uuid::new_v4();
        let webpush = Rc::new(RefCell::new(WebPushClient {
            uaid,
            uid: uid,
            flags,
            rx,
            message_month,
            connected_at,
            stats: SessionStatistics {
                uaid: uaid.simple().to_string(),
                uaid_reset: reset_uaid,
                existing_uaid: check_storage,
                connection_type: String::from("webpush"),
                host: host.clone(),
                ..Default::default()
            },
            ..Default::default()
        }));
        srv.connect_client(RegisteredClient { uaid, uid, tx });

        let response = ServerMessage::Hello {
            uaid: uaid.simple().to_string(),
            status: 200,
            use_webpush: Some(true),
            broadcasts: Service::into_hashmap(broadcasts),
        };
        let auth_state_machine = AuthClientState::start(
            vec![response],
            false,
            AuthClientData {
                srv: srv.clone(),
                ws,
                webpush: webpush.clone(),
                broadcast_services: broadcast_services.clone(),
            },
        );
        transition!(AwaitSessionComplete {
            auth_state_machine,
            srv,
            user_agent,
            webpush,
        })
    }

    fn poll_await_session_complete<'a>(
        session_complete: &'a mut RentToOwn<'a, AwaitSessionComplete<T>>,
    ) -> Poll<AfterAwaitSessionComplete, Error> {
        // xxx: handle error cases with maybe a log message?
        let _error = {
            match session_complete.auth_state_machine.poll() {
                Ok(Async::Ready(_)) => None,
                Ok(Async::NotReady) => return Ok(Async::NotReady),
                Err(e) => Some(e),
            }
        };

        let AwaitSessionComplete {
            srv,
            user_agent,
            webpush,
            ..
        } = session_complete.take();
        let mut webpush = webpush.borrow_mut();
        // If there's any notifications in the queue, move them to our unacked direct notifs
        webpush.rx.close();
        loop {
            match webpush.rx.poll() {
                Ok(Async::Ready(Some(msg))) => match msg {
                    ServerNotification::CheckStorage | ServerNotification::Disconnect => continue,
                    ServerNotification::Notification(notif) => {
                        webpush.unacked_direct_notifs.push(notif)
                    }
                },
                Ok(Async::Ready(None)) | Ok(Async::NotReady) | Err(_) => break,
            }
        }
        let now = ms_since_epoch();
        let elapsed = (now - webpush.connected_at) / 1_000;
        let parser = Parser::new();
        let (ua_result, metrics_os, metrics_browser) = parse_user_agent(&parser, &user_agent);
        srv.metrics
            .time_with_tags("ua.connection.lifespan", elapsed)
            .with_tag("ua_os_family", metrics_os)
            .with_tag("ua_browser_family", metrics_browser)
            .with_tag("host", &webpush.stats.host)
            .send();

        // If there's direct unack'd messages, they need to be saved out without blocking
        // here
        srv.disconnet_client(&webpush.uaid, &webpush.uid);
        let mut stats = webpush.stats.clone();
        let unacked_direct_notifs = webpush.unacked_direct_notifs.len();
        if unacked_direct_notifs > 0 {
            debug!("Writing direct notifications to storage");
            stats.direct_storage += unacked_direct_notifs as i32;
            let mut notifs = mem::replace(&mut webpush.unacked_direct_notifs, Vec::new());
            // Ensure we don't store these as legacy by setting a 0 as the sortkey_timestamp
            // That will ensure the Python side doesn't mark it as legacy during conversion and
            // still get the correct default us_time when saving.
            for notif in &mut notifs {
                notif.sortkey_timestamp = Some(0);
            }
            srv.handle.spawn(
                srv.ddb
                    .store_messages(&webpush.uaid, &webpush.message_month, notifs)
                    .then(|_| {
                        debug!("Finished saving unacked direct notifications");
                        Ok(())
                    }),
            );
        }

        // Log out the final stats message
        info!("Session";
        "uaid_hash" => &stats.uaid,
        "uaid_reset" => stats.uaid_reset,
        "existing_uaid" => stats.existing_uaid,
        "connection_type" => &stats.connection_type,
        "host" => &stats.host,
        "ua_name" => ua_result.name,
        "ua_os_family" => ua_result.os,
        "ua_os_ver" => ua_result.os_version,
        "ua_browser_family" => ua_result.vendor,
        "ua_browser_ver" => ua_result.version,
        "ua_category" => ua_result.category,
        "connection_time" => elapsed,
        "direct_acked" => stats.direct_acked,
        "direct_storage" => stats.direct_storage,
        "stored_retrieved" => stats.stored_retrieved,
        "stored_acked" => stats.stored_acked,
        "nacks" => stats.nacks,
        "registers" => stats.registers,
        "unregisters" => stats.unregisters,
        );
        transition!(UnAuthDone(()))
    }
}

#[derive(StateMachineFuture)]
pub enum AuthClientState<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    #[state_machine_future(start, transitions(DetermineAck, SendThenWait))]
    SendThenWait {
        remaining_data: Vec<ServerMessage>,
        poll_complete: bool,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(IncrementStorage, CheckStorage, AwaitDropUser,
                                       AwaitMigrateUser, AwaitInput))]
    DetermineAck { data: AuthClientData<T> },

    #[state_machine_future(transitions(DetermineAck, SendThenWait, AwaitInput, AwaitRegister,
                                       AwaitUnregister, AwaitDelete))]
    AwaitInput { data: AuthClientData<T> },

    #[state_machine_future(transitions(AwaitIncrementStorage))]
    IncrementStorage { data: AuthClientData<T> },

    #[state_machine_future(transitions(DetermineAck))]
    AwaitIncrementStorage {
        response: MyFuture<UpdateItemOutput>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(AwaitCheckStorage))]
    CheckStorage { data: AuthClientData<T> },

    #[state_machine_future(transitions(SendThenWait, DetermineAck))]
    AwaitCheckStorage {
        response: MyFuture<CheckStorageResponse>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(DetermineAck))]
    AwaitMigrateUser {
        response: MyFuture<()>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(AuthDone))]
    AwaitDropUser {
        response: MyFuture<()>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(SendThenWait))]
    AwaitRegister {
        channel_id: Uuid,
        response: MyFuture<RegisterResponse>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(SendThenWait))]
    AwaitUnregister {
        channel_id: Uuid,
        code: u32,
        response: MyFuture<bool>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(transitions(DetermineAck))]
    AwaitDelete {
        response: MyFuture<()>,
        data: AuthClientData<T>,
    },

    #[state_machine_future(ready)]
    AuthDone(()),

    #[state_machine_future(error)]
    AuthClientStateError(Error),
}

impl<T> PollAuthClientState<T> for AuthClientState<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
{
    fn poll_send_then_wait<'a>(
        send: &'a mut RentToOwn<'a, SendThenWait<T>>,
    ) -> Poll<AfterSendThenWait<T>, Error> {
        trace!("State: SendThenWait");
        let start_send = {
            let SendThenWait {
                ref mut remaining_data,
                poll_complete,
                ref mut data,
                ..
            } = **send;
            if poll_complete {
                try_ready!(data.ws.poll_complete());
                false
            } else if !remaining_data.is_empty() {
                let item = remaining_data.remove(0);
                let ret = data.ws.start_send(item).chain_err(|| "unable to send")?;
                match ret {
                    AsyncSink::Ready => true,
                    AsyncSink::NotReady(returned) => {
                        remaining_data.insert(0, returned);
                        return Ok(Async::NotReady);
                    }
                }
            } else {
                false
            }
        };

        let SendThenWait {
            data,
            remaining_data,
            ..
        } = send.take();
        if start_send {
            transition!(SendThenWait {
                remaining_data,
                poll_complete: true,
                data,
            });
        } else if !remaining_data.is_empty() {
            transition!(SendThenWait {
                remaining_data,
                poll_complete: false,
                data,
            });
        }
        transition!(DetermineAck { data })
    }

    fn poll_determine_ack<'a>(
        detack: &'a mut RentToOwn<'a, DetermineAck<T>>,
    ) -> Poll<AfterDetermineAck<T>, Error> {
        let DetermineAck { data } = detack.take();
        let webpush_rc = data.webpush.clone();
        let webpush = webpush_rc.borrow();
        let all_acked = !webpush.unacked_messages();
        if all_acked && webpush.flags.check && webpush.flags.increment_storage {
            transition!(IncrementStorage { data });
        } else if all_acked && webpush.flags.check {
            transition!(CheckStorage { data });
        } else if all_acked && webpush.flags.rotate_message_table {
            debug!("Triggering migration");
            let response = Box::new(data.srv.ddb.migrate_user(
                &webpush.uaid,
                &webpush.message_month,
                &data.srv.opts.current_message_month,
                &data.srv.opts.router_table_name,
            ));
            transition!(AwaitMigrateUser { response, data });
        } else if all_acked && webpush.flags.reset_uaid {
            let response = Box::new(
                data.srv
                    .ddb
                    .drop_uaid(&data.srv.opts.router_table_name, &webpush.uaid)
            );
            transition!(AwaitDropUser { response, data });
        }
        transition!(AwaitInput { data })
    }

    fn poll_await_input<'a>(
        await: &'a mut RentToOwn<'a, AwaitInput<T>>,
    ) -> Poll<AfterAwaitInput<T>, Error> {
        trace!("State: AwaitInput");
        let input = try_ready!(await.data.input_or_notif());
        let AwaitInput { data } = await.take();
        let webpush_rc = data.webpush.clone();
        let mut webpush = webpush_rc.borrow_mut();
        match input {
            Either::A(ClientMessage::BroadcastSubscribe { broadcasts }) => {
                let service_delta = {
                    let mut broadcast_services = data.broadcast_services.borrow_mut();
                    data.srv.client_service_add_service(
                        &mut broadcast_services,
                        &Service::from_hashmap(broadcasts),
                    )
                };
                if let Some(delta) = service_delta {
                    transition!(SendThenWait {
                        remaining_data: vec![
                            ServerMessage::Broadcast {
                                broadcasts: Service::into_hashmap(delta),
                            },
                        ],
                        poll_complete: false,
                        data,
                    });
                } else {
                    transition!(AwaitInput { data });
                }
            }
            Either::A(ClientMessage::Register {
                channel_id: channel_id_str,
                key,
            }) => {
                debug!("Got a register command";
                       "channel_id" => &channel_id_str);
                let channel_id =
                    Uuid::parse_str(&channel_id_str).chain_err(|| "Invalid channelID")?;
                if channel_id.hyphenated().to_string() != channel_id_str {
                    return Err("Bad UUID format, use lower case, dashed format".into());
                }

                let uaid = webpush.uaid;
                let message_month = webpush.message_month.clone();
                let srv = data.srv.clone();
                let fut = data.srv
                    .ddb
                    .register(&srv, &uaid, &channel_id, &message_month, key);
                transition!(AwaitRegister {
                    channel_id,
                    response: fut,
                    data,
                });
            }
            Either::A(ClientMessage::Unregister { channel_id, code }) => {
                debug!("Got a unregister command");
                // XXX: unregister should check the format of channel_id like
                // register does
                let uaid = webpush.uaid;
                let message_month = webpush.message_month.clone();
                let response =
                    Box::new(data.srv.ddb.unregister(&uaid, &channel_id, &message_month));
                transition!(AwaitUnregister {
                    channel_id,
                    code: code.unwrap_or(200),
                    response,
                    data,
                });
            }
            Either::A(ClientMessage::Nack { .. }) => {
                data.srv.metrics.incr("ua.command.nack").ok();
                webpush.stats.nacks += 1;
                transition!(AwaitInput { data });
            }
            Either::A(ClientMessage::Ack { updates }) => {
                data.srv.metrics.incr("ua.command.ack").ok();
                let mut fut: Option<MyFuture<()>> = None;
                for notif in &updates {
                    if let Some(pos) = webpush.unacked_direct_notifs.iter().position(|v| {
                        v.channel_id == notif.channel_id && v.version == notif.version
                    }) {
                        webpush.stats.direct_acked += 1;
                        webpush.unacked_direct_notifs.remove(pos);
                        continue;
                    };
                    if let Some(pos) = webpush.unacked_stored_notifs.iter().position(|v| {
                        v.channel_id == notif.channel_id && v.version == notif.version
                    }) {
                        webpush.stats.stored_acked += 1;
                        let message_month = webpush.message_month.clone();
                        let n = webpush.unacked_stored_notifs.remove(pos);
                        // Topic/legacy messages have no sortkey_timestamp
                        if n.sortkey_timestamp.is_none() {
                            fut = if let Some(call) = fut {
                                let my_fut =
                                    data.srv
                                        .ddb
                                        .delete_message(&message_month, &webpush.uaid, &n);
                                Some(Box::new(call.and_then(move |_| my_fut)))
                            } else {
                                Some(Box::new(data.srv.ddb.delete_message(
                                    &message_month,
                                    &webpush.uaid,
                                    &n,
                                )))
                            }
                        }
                        continue;
                    };
                }
                if let Some(my_fut) = fut {
                    transition!(AwaitDelete {
                        response: my_fut,
                        data,
                    });
                } else {
                    transition!(DetermineAck { data });
                }
            }
            Either::B(ServerNotification::Notification(notif)) => {
                if notif.ttl != 0 {
                    webpush.unacked_direct_notifs.push(notif.clone());
                }
                debug!("Got a notification to send, sending!");
                transition!(SendThenWait {
                    remaining_data: vec![ServerMessage::Notification(notif)],
                    poll_complete: false,
                    data,
                });
            }
            Either::B(ServerNotification::CheckStorage) => {
                webpush.flags.include_topic = true;
                webpush.flags.check = true;
                transition!(DetermineAck { data });
            }
            Either::B(ServerNotification::Disconnect) => {
                debug!("Got told to disconnect, connecting client has our uaid");
                Err("Repeat UAID disconnect".into())
            }
            _ => Err("Invalid message".into()),
        }
    }

    fn poll_increment_storage<'a>(
        increment_storage: &'a mut RentToOwn<'a, IncrementStorage<T>>,
    ) -> Poll<AfterIncrementStorage<T>, Error> {
        trace!("State: IncrementStorage");
        let webpush_rc = increment_storage.data.webpush.clone();
        let webpush = webpush_rc.borrow();
        let timestamp = webpush
            .unacked_stored_highest
            .ok_or("unacked_stored_highest unset")?
            .to_string();
        let response = Box::new(increment_storage.data.srv.ddb.increment_storage(
            &webpush.message_month,
            &webpush.uaid,
            &timestamp,
        ));
        transition!(AwaitIncrementStorage {
            response,
            data: increment_storage.take().data,
        })
    }

    fn poll_await_increment_storage<'a>(
        await_increment_storage: &'a mut RentToOwn<'a, AwaitIncrementStorage<T>>,
    ) -> Poll<AfterAwaitIncrementStorage<T>, Error> {
        trace!("State: AwaitIncrementStorage");
        try_ready!(await_increment_storage.response.poll());
        let AwaitIncrementStorage { data, .. } = await_increment_storage.take();
        let webpush = data.webpush.clone();
        webpush.borrow_mut().flags.increment_storage = false;
        transition!(DetermineAck { data })
    }

    fn poll_check_storage<'a>(
        check_storage: &'a mut RentToOwn<'a, CheckStorage<T>>,
    ) -> Poll<AfterCheckStorage<T>, Error> {
        trace!("State: CheckStorage");
        let CheckStorage { data } = check_storage.take();
        let response = Box::new({
            let webpush = data.webpush.borrow();
            data.srv.ddb.check_storage(
                &webpush.message_month.clone(),
                &webpush.uaid,
                webpush.flags.include_topic,
                webpush.unacked_stored_highest,
            )
        });
        transition!(AwaitCheckStorage { response, data })
    }

    fn poll_await_check_storage<'a>(
        await_check_storage: &'a mut RentToOwn<'a, AwaitCheckStorage<T>>,
    ) -> Poll<AfterAwaitCheckStorage<T>, Error> {
        trace!("State: AwaitCheckStorage");
        let (include_topic, mut messages, timestamp) =
            match try_ready!(await_check_storage.response.poll()) {
                CheckStorageResponse {
                    include_topic,
                    messages,
                    timestamp,
                } => (include_topic, messages, timestamp),
            };
        debug!("Got checkstorage response");

        let AwaitCheckStorage { data, .. } = await_check_storage.take();
        let webpush_rc = data.webpush.clone();
        let mut webpush = webpush_rc.borrow_mut();
        webpush.flags.include_topic = include_topic;
        debug!("Setting unacked stored highest to {:?}", timestamp);
        webpush.unacked_stored_highest = timestamp;
        if !messages.is_empty() {
            // Filter out TTL expired messages
            let now = sec_since_epoch() as u32;
            messages.retain(|ref msg| now < msg.ttl + msg.timestamp);
            webpush.flags.increment_storage = !include_topic && timestamp.is_some();
            // If there's still messages send them out
            if !messages.is_empty() {
                webpush
                    .unacked_stored_notifs
                    .extend(messages.iter().cloned());
                transition!(SendThenWait {
                    remaining_data: messages
                        .into_iter()
                        .map(ServerMessage::Notification)
                        .collect(),
                    poll_complete: false,
                    data,
                })
            } else {
                // No messages remaining
                transition!(DetermineAck { data })
            }
        } else {
            webpush.flags.check = false;
            transition!(DetermineAck { data })
        }
    }

    fn poll_await_migrate_user<'a>(
        await_migrate_user: &'a mut RentToOwn<'a, AwaitMigrateUser<T>>,
    ) -> Poll<AfterAwaitMigrateUser<T>, Error> {
        trace!("State: AwaitMigrateUser");
        try_ready!(await_migrate_user.response.poll());
        let AwaitMigrateUser { data, .. } = await_migrate_user.take();
        {
            let mut webpush = data.webpush.borrow_mut();
            webpush.message_month = data.srv.opts.current_message_month.clone();
            webpush.flags.rotate_message_table = false;
        }
        transition!(DetermineAck { data })
    }

    fn poll_await_drop_user<'a>(
        await_drop_user: &'a mut RentToOwn<'a, AwaitDropUser<T>>,
    ) -> Poll<AfterAwaitDropUser, Error> {
        trace!("State: AwaitDropUser");
        try_ready!(await_drop_user.response.poll());
        transition!(AuthDone(()))
    }

    fn poll_await_register<'a>(
        await_register: &'a mut RentToOwn<'a, AwaitRegister<T>>,
    ) -> Poll<AfterAwaitRegister<T>, Error> {
        trace!("State: AwaitRegister");
        let msg = match try_ready!(await_register.response.poll()) {
            RegisterResponse::Success { endpoint } => {
                let mut webpush = await_register.data.webpush.borrow_mut();
                await_register
                    .data
                    .srv
                    .metrics
                    .incr("ua.command.register")
                    .ok();
                webpush.stats.registers += 1;
                ServerMessage::Register {
                    channel_id: await_register.channel_id,
                    status: 200,
                    push_endpoint: endpoint,
                }
            }
            RegisterResponse::Error { error_msg, status } => {
                debug!("Got unregister fail, error: {}", error_msg);
                ServerMessage::Register {
                    channel_id: await_register.channel_id,
                    status,
                    push_endpoint: "".into(),
                }
            }
        };

        transition!(SendThenWait {
            remaining_data: vec![msg],
            poll_complete: false,
            data: await_register.take().data,
        })
    }

    fn poll_await_unregister<'a>(
        await_unregister: &'a mut RentToOwn<'a, AwaitUnregister<T>>,
    ) -> Poll<AfterAwaitUnregister<T>, Error> {
        trace!("State: AwaitUnRegister");
        let msg = if try_ready!(await_unregister.response.poll()) {
            debug!("Got the unregister response");
            let mut webpush = await_unregister.data.webpush.borrow_mut();
            webpush.stats.unregisters += 1;
            ServerMessage::Unregister {
                channel_id: await_unregister.channel_id,
                status: 200,
            }
        } else {
            debug!("Got unregister fail");
            ServerMessage::Unregister {
                channel_id: await_unregister.channel_id,
                status: 500,
            }
        };

        let AwaitUnregister { code, data, .. } = await_unregister.take();
        data.srv
            .metrics
            .incr_with_tags("ua.command.unregister")
            .with_tag("code", &code.to_string())
            .send();
        transition!(SendThenWait {
            remaining_data: vec![msg],
            poll_complete: false,
            data
        })
    }

    fn poll_await_delete<'a>(
        await_delete: &'a mut RentToOwn<'a, AwaitDelete<T>>,
    ) -> Poll<AfterAwaitDelete<T>, Error> {
        trace!("State: AwaitDelete");
        try_ready!(await_delete.response.poll());
        transition!(DetermineAck {
            data: await_delete.take().data,
        })
    }
}
