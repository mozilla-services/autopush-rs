use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::panic;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use cadence::StatsdClient;
use chrono::Utc;
use fernet::{Fernet, MultiFernet};
use futures::sync::oneshot;
use futures::{task, try_ready};
use futures::{Async, AsyncSink, Future, Poll, Sink, StartSend, Stream};
use hyper::{server::conn::Http, StatusCode};
use openssl::ssl::SslAcceptor;
use sentry::{self, capture_message};
use serde_json::{self, json};
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Handle, Timeout};
use tokio_tungstenite::{accept_hdr_async, WebSocketStream};
use tungstenite::handshake::server::Request;
use tungstenite::{self, Message};

use autopush_common::errors::{ApcError, ApcErrorKind, Result};
use autopush_common::logging;
use autopush_common::notification::Notification;

use crate::client::Client;
use crate::db::DynamoStorage;
use crate::megaphone::{
    Broadcast, BroadcastChangeTracker, BroadcastSubs, BroadcastSubsInit, MegaphoneAPIResponse,
};
use crate::server::dispatch::{Dispatch, RequestType};
use crate::server::metrics::metrics_from_opts;
use crate::server::protocol::{BroadcastValue, ClientMessage, ServerMessage};
use crate::server::rc::RcObject;
use crate::server::registry::ClientRegistry;
use crate::server::webpush_io::WebpushIo;
use crate::settings::Settings;
use crate::{http, timeout, MyFuture};

mod dispatch;
mod metrics;
pub mod protocol;
mod rc;
pub mod registry;
mod tls;
mod webpush_io;

const UAHEADER: &str = "User-Agent";

fn ito_dur(seconds: u32) -> Option<Duration> {
    if seconds == 0 {
        None
    } else {
        Some(Duration::new(seconds.into(), 0))
    }
}

fn fto_dur(seconds: f64) -> Option<Duration> {
    if seconds == 0.0 {
        None
    } else {
        Some(Duration::new(
            seconds as u64,
            (seconds.fract() * 1_000_000_000.0) as u32,
        ))
    }
}

// a signaler to shut down a tokio Core and its associated thread
struct ShutdownHandle(oneshot::Sender<()>, thread::JoinHandle<()>);

pub struct AutopushServer {
    opts: Arc<ServerOptions>,
    shutdown_handles: Cell<Option<Vec<ShutdownHandle>>>,
    _guard: Option<sentry::ClientInitGuard>,
}

impl AutopushServer {
    pub fn new(opts: ServerOptions) -> Self {
        let guard = if let Ok(dsn) = env::var("SENTRY_DSN") {
            let guard = sentry::init((
                dsn,
                sentry::ClientOptions {
                    release: sentry::release_name!(),
                    ..Default::default()
                },
            ));
            /*
            Sentry 0.29+ automatically enables `PanicIntegration`.
            see https://docs.rs/sentry-panic/latest/sentry_panic/
             */
            Some(guard)
        } else {
            None
        };
        Self {
            opts: Arc::new(opts),
            shutdown_handles: Cell::new(None),
            _guard: guard,
        }
    }

    pub fn start(&self) {
        logging::init_logging(!self.opts.human_logs).expect("init_logging failed");
        let handles = Server::start(&self.opts).expect("failed to start server");
        self.shutdown_handles.set(Some(handles));
    }

    /// Blocks execution of the calling thread until the helper thread with the
    /// tokio reactor has exited.
    pub fn stop(&self) -> Result<()> {
        let mut result = Ok(());
        if let Some(shutdown_handles) = self.shutdown_handles.take() {
            for ShutdownHandle(tx, thread) in shutdown_handles {
                let _ = tx.send(());
                if let Err(err) = thread.join() {
                    result = Err(ApcErrorKind::Thread(err).into());
                }
            }
        }
        logging::reset_logging();
        result
    }
}

pub struct ServerOptions {
    pub router_port: u16,
    pub port: u16,
    pub fernet: MultiFernet,
    pub ssl_key: Option<PathBuf>,
    pub ssl_cert: Option<PathBuf>,
    pub ssl_dh_param: Option<PathBuf>,
    pub open_handshake_timeout: Option<Duration>,
    pub auto_ping_interval: Duration,
    pub auto_ping_timeout: Duration,
    pub max_connections: Option<u32>,
    pub close_handshake_timeout: Option<Duration>,
    pub _message_table_name: String,
    pub _router_table_name: String,
    pub router_url: String,
    pub endpoint_url: String,
    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: Duration,
    pub human_logs: bool,
    pub msg_limit: u32,
}

impl ServerOptions {
    pub fn from_settings(settings: Settings) -> Result<Self> {
        let crypto_key = &settings.crypto_key;
        if !(crypto_key.starts_with('[') && crypto_key.ends_with(']')) {
            return Err(ApcErrorKind::GeneralError("Invalid AUTOPUSH_CRYPTO_KEY".into()).into());
        }
        let crypto_key = &crypto_key[1..crypto_key.len() - 1];
        debug!("Fernet keys: {:?}", &crypto_key);
        let fernets: Vec<Fernet> = crypto_key
            .split(',')
            .map(|s| s.trim().to_string())
            .map(|key| Fernet::new(&key).expect("Invalid AUTOPUSH_CRYPTO_KEY"))
            .collect();
        let fernet = MultiFernet::new(fernets);

        let router_url = settings.router_url();
        let endpoint_url = settings.endpoint_url();
        Ok(Self {
            port: settings.port,
            fernet,
            router_port: settings.router_port,
            statsd_host: if settings.statsd_host.is_empty() {
                None
            } else {
                Some(settings.statsd_host)
            },
            statsd_port: settings.statsd_port,
            _message_table_name: settings.message_tablename,
            _router_table_name: settings.router_tablename,
            router_url,
            endpoint_url,
            ssl_key: settings.router_ssl_key.map(PathBuf::from),
            ssl_cert: settings.router_ssl_cert.map(PathBuf::from),
            ssl_dh_param: settings.router_ssl_dh_param.map(PathBuf::from),
            auto_ping_interval: fto_dur(settings.auto_ping_interval)
                .expect("auto ping interval cannot be 0"),
            auto_ping_timeout: fto_dur(settings.auto_ping_timeout)
                .expect("auto ping timeout cannot be 0"),
            close_handshake_timeout: ito_dur(settings.close_handshake_timeout),
            max_connections: if settings.max_connections == 0 {
                None
            } else {
                Some(settings.max_connections)
            },
            open_handshake_timeout: ito_dur(5),
            megaphone_api_url: settings.megaphone_api_url,
            megaphone_api_token: settings.megaphone_api_token,
            megaphone_poll_interval: ito_dur(settings.megaphone_poll_interval)
                .expect("megaphone poll interval cannot be 0"),
            human_logs: settings.human_logs,
            msg_limit: settings.msg_limit,
        })
    }
}

/// The main AutoConnect server
pub struct Server {
    /// List of known Clients, mapped by UAID, for this node.
    pub clients: Arc<ClientRegistry>,
    /// Handle to the Broadcast change monitor
    broadcaster: RefCell<BroadcastChangeTracker>,
    /// Handle to the current Database Client
    pub ddb: DynamoStorage,
    /// Count of open cells
    open_connections: Cell<u32>,
    /// OBSOLETE
    tls_acceptor: Option<SslAcceptor>,
    /// Configuration options
    pub opts: Arc<ServerOptions>,
    /// tokio reactor core handle
    pub handle: Handle,
    /// analytics reporting
    pub metrics: Arc<StatsdClient>,
}

impl Server {
    /// Creates a new server handle used by Megaphone and other services.
    ///
    /// This will spawn a new server with the `opts` specified, spinning up a
    /// separate thread for the tokio reactor. The returned ShutdownHandles can
    /// be used to interact with it (e.g. shut it down).
    fn start(opts: &Arc<ServerOptions>) -> Result<Vec<ShutdownHandle>> {
        let mut shutdown_handles = vec![];

        let (inittx, initrx) = oneshot::channel();
        let (donetx, donerx) = oneshot::channel();

        let opts = opts.clone();
        let thread = thread::spawn(move || {
            let (srv, mut core) = match Server::new(&opts) {
                Ok(core) => {
                    inittx.send(None).unwrap();
                    core
                }
                Err(e) => return inittx.send(Some(e)).unwrap(),
            };

            // Internal HTTP server setup
            {
                let handle = core.handle();
                let addr = SocketAddr::from(([0, 0, 0, 0], srv.opts.router_port));
                let push_listener = TcpListener::bind(&addr, &handle).unwrap();
                let http = Http::new();
                let push_srv = push_listener.incoming().for_each(move |(socket, _)| {
                    handle.spawn(
                        http.serve_connection(socket, http::Push(Arc::clone(&srv.clients)))
                            .map(|_| ())
                            .map_err(|e| debug!("Http server connection error: {}", e)),
                    );
                    Ok(())
                });
                core.handle().spawn(push_srv.then(|res| {
                    debug!("Http server {:?}", res);
                    Ok(())
                }));
            }
            core.run(donerx).expect("Main Core run error");
        });

        match initrx.wait() {
            Ok(Some(e)) => Err(e),
            Ok(None) => {
                shutdown_handles.push(ShutdownHandle(donetx, thread));
                Ok(shutdown_handles)
            }
            Err(_) => panic::resume_unwind(thread.join().unwrap_err()),
        }
    }

    #[allow(clippy::single_char_add_str)]
    fn new(opts: &Arc<ServerOptions>) -> Result<(Rc<Server>, Core)> {
        let core = Core::new()?;
        let broadcaster = if let Some(ref megaphone_url) = opts.megaphone_api_url {
            let megaphone_token = opts
                .megaphone_api_token
                .as_ref()
                .expect("Megaphone API requires a Megaphone API Token to be set");
            BroadcastChangeTracker::with_api_broadcasts(megaphone_url, megaphone_token)
                .expect("Unable to initialize megaphone with provided URL")
        } else {
            BroadcastChangeTracker::new(Vec::new())
        };
        let metrics = Arc::new(metrics_from_opts(opts)?);

        let srv = Rc::new(Server {
            opts: opts.clone(),
            broadcaster: RefCell::new(broadcaster),
            ddb: DynamoStorage::from_opts(
                &opts._message_table_name,
                &opts._router_table_name,
                metrics.clone(),
            )?,
            clients: Arc::new(ClientRegistry::default()),
            open_connections: Cell::new(0),
            handle: core.handle(),
            tls_acceptor: tls::configure(opts),
            metrics,
        });
        let addr = SocketAddr::from(([0, 0, 0, 0], srv.opts.port));
        debug!("{:?}", &addr);
        let ws_listener = TcpListener::bind(&addr, &srv.handle)?;

        let handle = core.handle();
        let srv2 = srv.clone();
        let ws_srv =
            ws_listener
                .incoming()
                .map_err(ApcErrorKind::from)
                .for_each(move |(socket, addr)| {
                    // Make sure we're not handling too many clients before we start the
                    // websocket handshake.
                    let max = srv.opts.max_connections.unwrap_or(u32::max_value());
                    if srv.open_connections.get() >= max {
                        info!(
                            "dropping {} as we already have too many open connections",
                            addr
                        );
                        return Ok(());
                    }
                    srv.open_connections.set(srv.open_connections.get() + 1);

                    // Process TLS (if configured)
                    let socket = tls::accept(&srv, socket);

                    // Figure out if this is a websocket or a `/status` request,
                    let request = socket.and_then(Dispatch::new);

                    // Time out both the TLS accept (if any) along with the dispatch
                    // to figure out where we're going.
                    let request = timeout(request, srv.opts.open_handshake_timeout, &handle);
                    let srv2 = srv.clone();
                    let handle2 = handle.clone();

                    // Setup oneshot to extract the user-agent from the header callback
                    let (uatx, uarx) = oneshot::channel();
                    let callback = |req: &Request| {
                        if let Some(value) = req.headers.find_first(UAHEADER) {
                            let mut valstr = String::new();
                            for c in value.iter() {
                                let c = *c as char;
                                valstr.push(c);
                            }
                            debug!("Found user-agent string"; "user-agent" => valstr.as_str());
                            uatx.send(valstr).unwrap();
                        }
                        debug!("No agent string found");
                        Ok(None)
                    };

                    let client = request.and_then(move |(socket, request)| -> MyFuture<_> {
                        match request {
                            RequestType::Status => write_status(socket),
                            RequestType::LBHeartBeat => {
                                write_json(socket, StatusCode::OK, serde_json::Value::from(""))
                            }
                            RequestType::Version => write_version_file(socket),
                            RequestType::LogCheck => write_log_check(socket),
                            RequestType::Websocket => {
                                // Perform the websocket handshake on each
                                // connection, but don't let it take too long.
                                let ws = accept_hdr_async(socket, callback).map_err(|_e| {
                                    ApcErrorKind::GeneralError("failed to accept client".into())
                                });
                                let ws = timeout(ws, srv2.opts.open_handshake_timeout, &handle2);

                                // Once the handshake is done we'll start the main
                                // communication with the client, managing pings
                                // here and deferring to `Client` to start driving
                                // the internal state machine.
                                Box::new(
                                    ws.and_then(move |ws| {
                                        trace!("🏓 starting ping manager");
                                        PingManager::new(&srv2, ws, uarx).map_err(|_e| {
                                            ApcErrorKind::GeneralError(
                                                "failed to make ping handler".into(),
                                            )
                                            .into()
                                        })
                                    })
                                    .flatten(),
                                )
                            }
                        }
                    });

                    let srv = srv.clone();
                    handle.spawn(client.then(move |res| {
                        srv.open_connections.set(srv.open_connections.get() - 1);
                        if let Err(e) = res {
                            debug!("{}: {}", addr, e.to_string());
                        }
                        Ok(())
                    }));

                    Ok(())
                });

        if let Some(ref megaphone_url) = opts.megaphone_api_url {
            let megaphone_token = opts
                .megaphone_api_token
                .as_ref()
                .expect("Megaphone API requires a Megaphone API Token to be set");
            let fut = MegaphoneUpdater::new(
                megaphone_url,
                megaphone_token,
                opts.megaphone_poll_interval,
                &srv2,
            )
            .expect("Unable to start megaphone updater");
            core.handle().spawn(fut.then(|res| {
                trace!("📢 megaphone result: {:?}", res.map(drop));
                Ok(())
            }));
        }
        core.handle().spawn(ws_srv.then(|res| {
            debug!("srv res: {:?}", res.map(drop));
            Ok(())
        }));

        Ok((srv2, core))
    }

    /// Initialize broadcasts for a newly connected client
    pub fn broadcast_init(
        &self,
        desired_broadcasts: &[Broadcast],
    ) -> (BroadcastSubs, HashMap<String, BroadcastValue>) {
        trace!("📢Initialized broadcasts");
        let bc = self.broadcaster.borrow();
        let BroadcastSubsInit(broadcast_subs, broadcasts) = bc.broadcast_delta(desired_broadcasts);
        let mut response = Broadcast::vec_into_hashmap(broadcasts);
        let missing = bc.missing_broadcasts(desired_broadcasts);
        if !missing.is_empty() {
            response.insert(
                "errors".to_string(),
                BroadcastValue::Nested(Broadcast::vec_into_hashmap(missing)),
            );
        }
        (broadcast_subs, response)
    }

    /// Calculate whether there's new broadcast versions to go out
    pub fn broadcast_delta(&self, broadcast_subs: &mut BroadcastSubs) -> Option<Vec<Broadcast>> {
        trace!("📢 Checking broadcast_delta");
        self.broadcaster.borrow().change_count_delta(broadcast_subs)
    }

    /// Process a broadcast list, adding new broadcasts to be tracked and locating missing ones
    /// Returns an appropriate response for use by the prototocol
    pub fn process_broadcasts(
        &self,
        broadcast_subs: &mut BroadcastSubs,
        broadcasts: &[Broadcast],
    ) -> Option<HashMap<String, BroadcastValue>> {
        let bc = self.broadcaster.borrow();
        let mut response: HashMap<String, BroadcastValue> = HashMap::new();
        let missing = bc.missing_broadcasts(broadcasts);
        if !missing.is_empty() {
            response.insert(
                "errors".to_string(),
                BroadcastValue::Nested(Broadcast::vec_into_hashmap(missing)),
            );
        }
        if let Some(delta) = bc.subscribe_to_broadcasts(broadcast_subs, broadcasts) {
            response.extend(Broadcast::vec_into_hashmap(delta));
        };
        if response.is_empty() {
            None
        } else {
            Some(response)
        }
    }
}

/*
STATE MACHINE
*/
enum MegaphoneState {
    Waiting,
    Requesting(MyFuture<MegaphoneAPIResponse>),
}

struct MegaphoneUpdater {
    srv: Rc<Server>,
    api_url: String,
    api_token: String,
    state: MegaphoneState,
    timeout: Timeout,
    poll_interval: Duration,
    client: reqwest::r#async::Client,
}

impl MegaphoneUpdater {
    fn new(
        uri: &str,
        token: &str,
        poll_interval: Duration,
        srv: &Rc<Server>,
    ) -> io::Result<MegaphoneUpdater> {
        let client = reqwest::r#async::Client::builder()
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
    type Item = ();
    type Error = ApcError;

    fn poll(&mut self) -> Poll<(), ApcError> {
        loop {
            let new_state = match self.state {
                MegaphoneState::Waiting => {
                    try_ready!(self.timeout.poll());
                    trace!("📢Sending megaphone API request");
                    let fut = self
                        .client
                        .get(&self.api_url)
                        .header("Authorization", self.api_token.clone())
                        .send()
                        .and_then(|response| response.error_for_status())
                        .and_then(|mut response| response.json())
                        .map_err(|_| {
                            ApcErrorKind::GeneralError(
                                "Unable to query/decode the API query".into(),
                            )
                            .into()
                        });
                    MegaphoneState::Requesting(Box::new(fut))
                }
                MegaphoneState::Requesting(ref mut response) => {
                    let at = Instant::now() + self.poll_interval;
                    match response.poll() {
                        Ok(Async::Ready(MegaphoneAPIResponse { broadcasts })) => {
                            trace!("📢Fetched broadcasts: {:?}", broadcasts);
                            let mut broadcaster = self.srv.broadcaster.borrow_mut();
                            for srv in Broadcast::from_hashmap(broadcasts) {
                                let vv = broadcaster.add_broadcast(srv);
                                trace!("📢   add_broadcast = {}", vv);
                                // TODO: Notify that Ping required?
                            }
                        }
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(error) => {
                            error!("📢Failed to get response, queue again {error:?}");
                            capture_message(
                                &format!("Failed to get response, queue again {error:?}"),
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

/*
STATE MACHINE
*/
enum CloseState<T> {
    Exchange(T),
    Closing,
}

struct PingManager {
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
        trace!("🏓Ping interval {:?}", &srv.opts.auto_ping_interval);
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
    type Item = ();
    type Error = ApcError;

    fn poll(&mut self) -> Poll<(), ApcError> {
        let mut socket = self.socket.borrow_mut();
        loop {
            trace!("🏓 PingManager Poll loop");
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
                    trace!("🏓 Time to ping");
                    // If we just sent a broadcast, reset the ping interval and clear the delta
                    if socket.broadcast_delta.is_some() {
                        trace!("📢 Pending");
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
                    trace!(
                        "🏓Checking pong timeout:{} pong recv'd:{}",
                        socket.ws_pong_timeout,
                        socket.ws_pong_received
                    );
                    debug_assert!(!socket.ws_pong_timeout);
                    debug_assert!(!socket.ws_pong_received);
                    match self.timeout.poll()? {
                        Async::Ready(()) => {
                            trace!("🏓scheduling a ws ping to get sent");
                            socket.ws_ping = true;
                        }
                        Async::NotReady => {
                            trace!("🏓not ready yet");
                            break;
                        }
                    }
                }
                WaitingFor::Pong => {
                    if socket.ws_pong_received {
                        // If we received a pong, then switch us back to waiting
                        // to send out a ping
                        trace!("🏓ws pong received, going back to sending a ping");
                        debug_assert!(!socket.ws_pong_timeout);
                        let at = Instant::now() + self.srv.opts.auto_ping_interval;
                        self.timeout.reset(at);
                        self.waiting = WaitingFor::SendPing;
                        socket.ws_pong_received = false;
                    } else if socket.ws_pong_timeout {
                        // If our socket is waiting to deliver a pong timeout,
                        // then no need to keep checking the timer and we can
                        // keep going
                        trace!("🏓waiting for socket to see ws pong timed out");
                        break;
                    } else if self.timeout.poll()?.is_ready() {
                        // We may not actually be reading messages from the
                        // websocket right now, could have been waiting on
                        // something else. Instead of immediately returning an
                        // error here wait for the stream to return `NotReady`
                        // when looking for messages, as then we're extra sure
                        // that no pong was received after this timeout elapsed.
                        trace!("🏓waited too long for a ws pong");
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
                        return Err(ApcErrorKind::GeneralError(
                            "close handshake took too long".into(),
                        )
                        .into());
                    }
                }
            }
        }

        // Be sure to always flush out any buffered messages/pings
        socket.poll_complete().map_err(|_e| {
            ApcErrorKind::GeneralError("failed routine `poll_complete` call".into())
        })?;
        drop(socket);

        // At this point looks our state of ping management A-OK, so try to
        // make progress on our client, and when done with that execute the
        // closing handshake.
        loop {
            match self.client {
                CloseState::Exchange(ref mut client) => try_ready!(client.poll()),
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

// Wrapper struct to take a Sink/Stream of `Message` to a Sink/Stream of
// `ClientMessage` and `ServerMessage`.
struct WebpushSocket<T> {
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

    fn send_ws_ping(&mut self) -> Poll<(), ApcError>
    where
        T: Sink<SinkItem = Message>,
        ApcError: From<T::SinkError>,
    {
        trace!("🏓 checking ping");
        if self.ws_ping {
            trace!("🏓 Ping present");
            let msg = if let Some(broadcasts) = self.broadcast_delta.clone() {
                trace!("🏓sending a broadcast delta");
                let server_msg = ServerMessage::Broadcast {
                    broadcasts: Broadcast::vec_into_hashmap(broadcasts),
                };
                let s = server_msg.to_json()?;
                Message::Text(s)
            } else {
                trace!("🏓sending a ws ping");
                Message::Ping(Vec::new())
            };
            match self.inner.start_send(msg)? {
                AsyncSink::Ready => {
                    trace!("🏓ws ping sent");
                    self.ws_ping = false;
                }
                AsyncSink::NotReady(_) => {
                    trace!("🏓ws ping not ready to be sent");
                    return Ok(Async::NotReady);
                }
            }
        } else {
            trace!("🏓No Ping");
        }
        Ok(Async::Ready(()))
    }
}

impl<T> Stream for WebpushSocket<T>
where
    T: Stream<Item = Message>,
    ApcError: From<T::Error>,
{
    type Item = ClientMessage;
    type Error = ApcError;

    fn poll(&mut self) -> Poll<Option<ClientMessage>, ApcError> {
        loop {
            let msg = match self.inner.poll()? {
                Async::Ready(Some(msg)) => msg,
                Async::Ready(None) => return Ok(None.into()),
                Async::NotReady => {
                    // If we don't have any more messages and our pong timeout
                    // elapsed (set above) then this is where we start
                    // triggering errors.
                    if self.ws_pong_timeout {
                        return Err(ApcErrorKind::PongTimeout.into());
                    }
                    return Ok(Async::NotReady);
                }
            };
            match msg {
                Message::Text(ref s) => {
                    trace!("🚟 text message {}", s);
                    let msg = s
                        .parse()
                        .map_err(|_e| ApcErrorKind::InvalidClientMessage(s.to_owned()))?;
                    return Ok(Some(msg).into());
                }

                Message::Binary(_) => {
                    return Err(
                        ApcErrorKind::InvalidClientMessage("binary content".to_string()).into(),
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
                    task::current().notify();
                }

                Message::Close(_) => return Err(tungstenite::Error::ConnectionClosed.into()),
            }
        }
    }
}

impl<T> Sink for WebpushSocket<T>
where
    T: Sink<SinkItem = Message>,
    ApcError: From<T::SinkError>,
{
    type SinkItem = ServerMessage;
    type SinkError = ApcError;

    fn start_send(&mut self, msg: ServerMessage) -> StartSend<ServerMessage, ApcError> {
        if self.send_ws_ping()?.is_not_ready() {
            return Ok(AsyncSink::NotReady(msg));
        }
        let s = msg
            .to_json()
            .map_err(|_e| ApcErrorKind::GeneralError("failed to serialize".into()))?;
        match self.inner.start_send(Message::Text(s))? {
            AsyncSink::Ready => Ok(AsyncSink::Ready),
            AsyncSink::NotReady(_) => Ok(AsyncSink::NotReady(msg)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), ApcError> {
        try_ready!(self.send_ws_ping());
        Ok(self.inner.poll_complete()?)
    }

    fn close(&mut self) -> Poll<(), ApcError> {
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

/// Return a static copy of `version.json` from compile time.
pub fn write_version_file(socket: WebpushIo) -> MyFuture<()> {
    write_json(
        socket,
        StatusCode::OK,
        serde_json::Value::from(include_str!("../../../version.json")),
    )
}

fn write_log_check(socket: WebpushIo) -> MyFuture<()> {
    let status = StatusCode::IM_A_TEAPOT;
    let code: u16 = status.into();

    error!("Test Critical Message";
           "status_code" => code,
           "errno" => 0,
    );
    thread::spawn(|| {
        panic!("LogCheck");
    });

    write_json(
        socket,
        StatusCode::IM_A_TEAPOT,
        json!({
                "code": code,
                "errno": 999,
                "error": "Test Failure",
                "mesage": "FAILURE:Success",
        }),
    )
}

fn write_json(socket: WebpushIo, status: StatusCode, body: serde_json::Value) -> MyFuture<()> {
    let body = body.to_string();
    let data = format!(
        "\
         HTTP/1.1 {status}\r\n\
         Server: webpush\r\n\
         Date: {date}\r\n\
         Content-Length: {len}\r\n\
         Content-Type: application/json\r\n\
         \r\n\
         {body}\
         ",
        status = status,
        date = Utc::now().to_rfc2822(),
        len = body.len(),
        body = body,
    );
    Box::new(
        tokio_io::io::write_all(socket, data.into_bytes())
            .map(|_| ())
            .map_err(|_e| {
                ApcErrorKind::GeneralError("failed to write status response".into()).into()
            }),
    )
}
