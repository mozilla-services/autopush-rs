use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::default::Default;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::panic;
use std::panic::PanicInfo;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use base64;
use cadence::StatsdClient;
use fernet::{Fernet, MultiFernet};
use futures::sync::oneshot;
use futures::task;
use futures::{Async, AsyncSink, Future, Poll, Sink, StartSend, Stream};
use hex;
use hyper::server::Http;
use hyper::{self, header, StatusCode};
use openssl::hash;
use openssl::ssl::SslAcceptor;
use reqwest;
use sentry;
use serde_json;
use time;
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Handle, Timeout};
use tokio_io;
use tokio_tungstenite::{accept_hdr_async, WebSocketStream};
use tungstenite::handshake::server::Request;
use tungstenite::Message;
use uuid::Uuid;

use client::{Client, RegisteredClient};
use db::DynamoStorage;
use errors::*;
use errors::{Error, Result};
use http;
use logging;
use protocol::{ClientMessage, Notification, ServerMessage, ServerNotification};
use server::dispatch::{Dispatch, RequestType};
use server::metrics::metrics_from_opts;
use server::webpush_io::WebpushIo;
use settings::Settings;
use util::megaphone::{
    ClientServices, MegaphoneAPIResponse, Service, ServiceChangeTracker, ServiceClientInit,
};
use util::{timeout, RcObject};

mod dispatch;
mod metrics;
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
}

impl AutopushServer {
    pub fn new(opts: ServerOptions) -> Self {
        Self {
            opts: Arc::new(opts),
            shutdown_handles: Cell::new(None),
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
                    result = Err(From::from(ErrorKind::Thread(err)));
                }
            }
        }
        logging::reset_logging();
        result
    }
}

pub struct ServerOptions {
    pub debug: bool,
    pub router_port: u16,
    pub port: u16,
    fernet: MultiFernet,
    pub ssl_key: Option<PathBuf>,
    pub ssl_cert: Option<PathBuf>,
    pub ssl_dh_param: Option<PathBuf>,
    pub open_handshake_timeout: Option<Duration>,
    pub auto_ping_interval: Duration,
    pub auto_ping_timeout: Duration,
    pub max_connections: Option<u32>,
    pub close_handshake_timeout: Option<Duration>,
    pub message_table_names: Vec<String>,
    pub current_message_month: String,
    pub router_table_name: String,
    pub router_url: String,
    pub endpoint_url: String,
    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: Duration,
    pub human_logs: bool,
}

impl ServerOptions {
    pub fn from_settings(settings: Settings) -> Result<Self> {
        let fernets: Vec<Fernet> = settings
            .crypto_key
            .split(',')
            .map(|s| s.trim().to_string())
            .map(|key| Fernet::new(&key).expect("Invalid key supplied"))
            .collect();
        let fernet = MultiFernet::new(fernets);
        let ddb = DynamoStorage::new();
        let message_table_names = ddb
            .list_message_tables(&settings.message_tablename)
            .expect("Failed to locate message tables");
        let router_url = settings.router_url();
        let endpoint_url = settings.endpoint_url();
        let mut opts = Self {
            debug: settings.debug,
            port: settings.port,
            fernet,
            router_port: settings.router_port,
            statsd_host: if settings.statsd_host.is_empty() {
                None
            } else {
                Some(settings.statsd_host)
            },
            statsd_port: settings.statsd_port,
            message_table_names,
            current_message_month: "".to_string(),
            router_table_name: settings.router_tablename,
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
        };
        opts.message_table_names.sort_unstable();
        opts.current_message_month = opts
            .message_table_names
            .last()
            .expect("No last message month found")
            .to_string();
        Ok(opts)
    }
}

pub struct Server {
    uaids: RefCell<HashMap<Uuid, RegisteredClient>>,
    broadcaster: RefCell<ServiceChangeTracker>,
    pub ddb: DynamoStorage,
    open_connections: Cell<u32>,
    tls_acceptor: Option<SslAcceptor>,
    pub opts: Arc<ServerOptions>,
    pub handle: Handle,
    pub metrics: StatsdClient,
}

impl Server {
    /// Creates a new server handle to send to python.
    ///
    /// This will spawn a new server with the `opts` specified, spinning up a
    /// separate thread for the tokio reactor. The returned ShutdownHandles can
    /// be used to interact with it (e.g. shut it down).
    fn start(opts: &Arc<ServerOptions>) -> Result<Vec<ShutdownHandle>> {
        let mut shutdown_handles = vec![];
        if let Some(handle) = Server::start_sentry()? {
            shutdown_handles.push(handle);
        }

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
                let http = Http::<hyper::Chunk>::new();
                let push_srv = push_listener.incoming().for_each(move |(socket, _)| {
                    handle.spawn(
                        http.serve_connection(socket, http::Push(srv.clone()))
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

    /// Setup Sentry logging if a SENTRY_DSN exists
    fn start_sentry() -> Result<Option<ShutdownHandle>> {
        let creds = match env::var("SENTRY_DSN") {
            Ok(dsn) => dsn.parse::<sentry::SentryCredential>()?,
            Err(_) => return Ok(None),
        };

        // Spin up a new thread with a new reactor core for the sentry handler
        let (donetx, donerx) = oneshot::channel();
        let thread = thread::spawn(move || {
            let mut core = Core::new().expect("Unable to create core");
            let sentry = sentry::Sentry::from_settings(core.handle(), Default::default(), creds);
            // Get the prior panic hook
            let hook = panic::take_hook();
            sentry.register_panic_handler(Some(move |info: &PanicInfo| -> () {
                hook(info);
            }));
            core.run(donerx).expect("Sentry Core run error");
        });

        Ok(Some(ShutdownHandle(donetx, thread)))
    }

    fn new(opts: &Arc<ServerOptions>) -> Result<(Rc<Server>, Core)> {
        let core = Core::new()?;
        let broadcaster = if let Some(ref megaphone_url) = opts.megaphone_api_url {
            let megaphone_token = opts
                .megaphone_api_token
                .as_ref()
                .expect("Megaphone API requires a Megaphone API Token to be set");
            ServiceChangeTracker::with_api_services(megaphone_url, megaphone_token)
                .expect("Unable to initialize megaphone with provided URL")
        } else {
            ServiceChangeTracker::new(Vec::new())
        };
        let srv = Rc::new(Server {
            opts: opts.clone(),
            broadcaster: RefCell::new(broadcaster),
            ddb: DynamoStorage::new(),
            uaids: RefCell::new(HashMap::new()),
            open_connections: Cell::new(0),
            handle: core.handle(),
            tls_acceptor: tls::configure(opts),
            metrics: metrics_from_opts(opts)?,
        });
        let addr = SocketAddr::from(([0, 0, 0, 0], srv.opts.port));
        let ws_listener = TcpListener::bind(&addr, &srv.handle)?;

        let handle = core.handle();
        let srv2 = srv.clone();
        let ws_srv = ws_listener
            .incoming()
            .map_err(Error::from)
            .for_each(move |(socket, addr)| {
                // Make sure we're not handling too many clients before we start the
                // websocket handshake.
                let max = srv.opts.max_connections.unwrap_or(u32::max_value());
                if srv.open_connections.get() >= max {
                    info!(
                        "dropping {} as we already have too many open \
                         connections",
                        addr
                    );
                    return Ok(());
                }
                srv.open_connections.set(srv.open_connections.get() + 1);

                // TODO: TCP socket options here?

                // Process TLS (if configured)
                let socket = tls::accept(&srv, socket);

                // Figure out if this is a websocket or a `/status` request,
                let request = socket.and_then(Dispatch::new);

                // Time out both the TLS accept (if any) along with the dispatch
                // to figure out where we're going.
                let request = timeout(request, srv.opts.open_handshake_timeout, &handle);
                let srv2 = srv.clone();
                let handle2 = handle.clone();

                let host = format!("{}", addr.ip());

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
                        RequestType::LogCheck => write_log_check(socket),
                        RequestType::Websocket => {
                            // Perform the websocket handshake on each
                            // connection, but don't let it take too long.
                            let ws = accept_hdr_async(socket, callback)
                                .chain_err(|| "failed to accept client");
                            let ws = timeout(ws, srv2.opts.open_handshake_timeout, &handle2);

                            // Once the handshake is done we'll start the main
                            // communication with the client, managing pings
                            // here and deferring to `Client` to start driving
                            // the internal state machine.
                            Box::new(
                                ws.and_then(move |ws| {
                                    PingManager::new(&srv2, ws, uarx, host)
                                        .chain_err(|| "failed to make ping handler")
                                }).flatten(),
                            )
                        }
                    }
                });

                let srv = srv.clone();
                handle.spawn(client.then(move |res| {
                    srv.open_connections.set(srv.open_connections.get() - 1);
                    if let Err(e) = res {
                        let mut error = e.to_string();
                        for err in e.iter().skip(1) {
                            error.push_str("\n");
                            error.push_str(&err.to_string());
                        }
                        debug!("{}: {}", addr, error);
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
            ).expect("Unable to start megaphone updater");
            core.handle().spawn(fut.then(|res| {
                debug!("megaphone result: {:?}", res.map(drop));
                Ok(())
            }));
        }
        core.handle().spawn(ws_srv.then(|res| {
            debug!("srv res: {:?}", res.map(drop));
            Ok(())
        }));

        Ok((srv2, core))
    }

    /// Create an v1 or v2 WebPush endpoint from the identifiers
    ///
    /// Both endpoints use bytes instead of hex to reduce ID length.
    //  v1 is the uaid + chid
    //  v2 is the uaid + chid + sha256(key).bytes
    pub fn make_endpoint(&self, uaid: &Uuid, chid: &Uuid, key: Option<String>) -> Result<String> {
        let root = format!("{}/wpush/", self.opts.endpoint_url);
        let mut base = hex::decode(uaid.simple().to_string()).chain_err(|| "Error decoding")?;
        base.extend(hex::decode(chid.simple().to_string()).chain_err(|| "Error decoding")?);
        if let Some(k) = key {
            let raw_key = base64::decode_config(&k, base64::URL_SAFE)
                .chain_err(|| "Error encrypting payload")?;
            let key_digest = hash::hash(hash::MessageDigest::sha256(), &raw_key)
                .chain_err(|| "Error creating message digest for key")?;
            base.extend(key_digest.iter());
            let encrypted = self
                .opts
                .fernet
                .encrypt(&base)
                .trim_matches('=')
                .to_string();
            Ok(format!("{}v2/{}", root, encrypted))
        } else {
            let encrypted = self
                .opts
                .fernet
                .encrypt(&base)
                .trim_matches('=')
                .to_string();
            Ok(format!("{}v1/{}", root, encrypted))
        }
    }

    /// Informs this server that a new `client` has connected
    ///
    /// For now just registers internal state by keeping track of the `client`,
    /// namely its channel to send notifications back.
    pub fn connect_client(&self, client: RegisteredClient) {
        debug!("Connecting a client!");
        if let Some(client) = self.uaids.borrow_mut().insert(client.uaid, client) {
            // Drop existing connection
            let result = client.tx.unbounded_send(ServerNotification::Disconnect);
            if result.is_ok() {
                debug!("Told client to disconnect as a new one wants to connect");
            }
        }
    }

    /// A notification has come for the uaid
    pub fn notify_client(&self, uaid: Uuid, notif: Notification) -> Result<()> {
        let uaids = self.uaids.borrow();
        if let Some(client) = uaids.get(&uaid) {
            debug!("Found a client to deliver a notification to");
            let result = client
                .tx
                .unbounded_send(ServerNotification::Notification(notif));
            if result.is_ok() {
                debug!("Dropped notification in queue");
                return Ok(());
            }
        }
        Err("User not connected".into())
    }

    /// A check for notification command has come for the uaid
    pub fn check_client_storage(&self, uaid: Uuid) -> Result<()> {
        let uaids = self.uaids.borrow();
        if let Some(client) = uaids.get(&uaid) {
            let result = client.tx.unbounded_send(ServerNotification::CheckStorage);
            if result.is_ok() {
                debug!("Told client to check storage");
                return Ok(());
            }
        }
        Err("User not connected".into())
    }

    /// The client specified by `uaid` has disconnected.
    pub fn disconnet_client(&self, uaid: &Uuid, uid: &Uuid) {
        debug!("Disconnecting client!");
        let mut uaids = self.uaids.borrow_mut();
        let client_exists = uaids.get(uaid).map_or(false, |client| client.uid == *uid);
        if client_exists {
            uaids.remove(uaid).expect("Couldn't remove client?");
        }
    }

    /// Generate a new service client list for a newly connected client
    pub fn broadcast_init(&self, services: &[Service]) -> ServiceClientInit {
        debug!("Initialized broadcast services");
        self.broadcaster.borrow().service_delta(services)
    }

    /// Calculate whether there's new service versions to go out
    pub fn broadcast_delta(&self, client_services: &mut ClientServices) -> Option<Vec<Service>> {
        self.broadcaster
            .borrow()
            .change_count_delta(client_services)
    }

    /// Add services to be tracked by a client
    pub fn client_service_add_service(
        &self,
        client_services: &mut ClientServices,
        services: &[Service],
    ) -> Option<Vec<Service>> {
        self.broadcaster
            .borrow()
            .client_service_add_service(client_services, services)
    }
}

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
    client: reqwest::unstable::async::Client,
}

impl MegaphoneUpdater {
    fn new(
        uri: &str,
        token: &str,
        poll_interval: Duration,
        srv: &Rc<Server>,
    ) -> io::Result<MegaphoneUpdater> {
        let client = reqwest::unstable::async::Client::builder()
            .timeout(Duration::from_secs(1))
            .build(&srv.handle)
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
    type Error = Error;

    fn poll(&mut self) -> Poll<(), Error> {
        loop {
            let new_state = match self.state {
                MegaphoneState::Waiting => {
                    try_ready!(self.timeout.poll());
                    debug!("Sending megaphone API request");
                    let fut = self
                        .client
                        .get(&self.api_url)
                        .header(header::Authorization(self.api_token.clone()))
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
                            for srv in Service::from_hashmap(broadcasts) {
                                broadcaster.add_service(srv);
                            }
                        }
                        Ok(Async::NotReady) => return Ok(Async::NotReady),
                        Err(_) => {
                            // TODO: Flag sentry that we can't poll megaphone API
                            debug!("Failed to get response, queue again");
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
        host: String,
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
            client: CloseState::Exchange(Client::new(socket, srv, uarx, host)),
            srv: srv.clone(),
        })
    }
}

impl Future for PingManager {
    type Item = ();
    type Error = Error;

    fn poll(&mut self) -> Poll<(), Error> {
        let mut socket = self.socket.borrow_mut();
        loop {
            if socket.ping {
                // Don't check if we already have a delta to broadcast
                if socket.broadcast_delta.is_none() {
                    // Determine if we can do a broadcast check, we need a connected webpush client
                    if let CloseState::Exchange(ref mut client) = self.client {
                        if let Some(delta) = client.broadcast_delta() {
                            socket.broadcast_delta = Some(delta);
                        }
                    }
                }

                if socket.send_ping()?.is_ready() {
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
            debug_assert!(!socket.ping);
            match self.waiting {
                WaitingFor::SendPing => {
                    debug_assert!(!socket.pong_timeout);
                    debug_assert!(!socket.pong_received);
                    match self.timeout.poll()? {
                        Async::Ready(()) => {
                            debug!("scheduling a ping to get sent");
                            socket.ping = true;
                        }
                        Async::NotReady => break,
                    }
                }
                WaitingFor::Pong => {
                    if socket.pong_received {
                        // If we received a pong, then switch us back to waiting
                        // to send out a ping
                        debug!("pong received, going back to sending a ping");
                        debug_assert!(!socket.pong_timeout);
                        let at = Instant::now() + self.srv.opts.auto_ping_interval;
                        self.timeout.reset(at);
                        self.waiting = WaitingFor::SendPing;
                        socket.pong_received = false;
                    } else if socket.pong_timeout {
                        // If our socket is waiting to deliver a pong timeout,
                        // then no need to keep checking the timer and we can
                        // keep going
                        debug!("waiting for socket to see pong timed out");
                        break;
                    } else if self.timeout.poll()?.is_ready() {
                        // We may not actually be reading messages from the
                        // websocket right now, could have been waiting on
                        // something else. Instead of immediately returning an
                        // error here wait for the stream to return `NotReady`
                        // when looking for messages, as then we're extra sure
                        // that no pong was received after this timeout elapsed.
                        debug!("waited too long for a pong");
                        socket.pong_timeout = true;
                    } else {
                        break;
                    }
                }
                WaitingFor::Close => {
                    debug_assert!(!socket.pong_timeout);
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
        socket
            .poll_complete()
            .chain_err(|| "failed routine `poll_complete` call")?;
        drop(socket);

        // At this point looks our state of ping management A-OK, so try to
        // make progress on our client, and when done with that execute the
        // closing handshake.
        loop {
            match self.client {
                CloseState::Exchange(ref mut client) => try_ready!(client.poll()),
                CloseState::Closing => return Ok(self.socket.borrow_mut().close()?),
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
    pong_received: bool,
    ping: bool,
    pong_timeout: bool,
    broadcast_delta: Option<Vec<Service>>,
}

impl<T> WebpushSocket<T> {
    fn new(t: T) -> WebpushSocket<T> {
        WebpushSocket {
            inner: t,
            pong_received: false,
            ping: false,
            pong_timeout: false,
            broadcast_delta: None,
        }
    }

    fn send_ping(&mut self) -> Poll<(), Error>
    where
        T: Sink<SinkItem = Message>,
        Error: From<T::SinkError>,
    {
        if self.ping {
            let msg = if let Some(broadcasts) = self.broadcast_delta.clone() {
                debug!("sending a broadcast delta");
                let server_msg = ServerMessage::Broadcast {
                    broadcasts: Service::into_hashmap(broadcasts),
                };
                let s = serde_json::to_string(&server_msg).chain_err(|| "failed to serialize")?;
                Message::Text(s)
            } else {
                debug!("sending a ping");
                Message::Ping(Vec::new())
            };
            match self.inner.start_send(msg)? {
                AsyncSink::Ready => {
                    debug!("ping sent");
                    self.ping = false;
                }
                AsyncSink::NotReady(_) => {
                    debug!("ping not ready to be sent");
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
    Error: From<T::Error>,
{
    type Item = ClientMessage;
    type Error = Error;

    fn poll(&mut self) -> Poll<Option<ClientMessage>, Error> {
        loop {
            let msg = match self.inner.poll()? {
                Async::Ready(Some(msg)) => msg,
                Async::Ready(None) => return Ok(None.into()),
                Async::NotReady => {
                    // If we don't have any more messages and our pong timeout
                    // elapsed (set above) then this is where we start
                    // triggering errors.
                    if self.pong_timeout {
                        return Err("failed to receive a pong in time".into());
                    }
                    return Ok(Async::NotReady);
                }
            };
            match msg {
                Message::Text(ref s) => {
                    trace!("text message {}", s);
                    let msg = serde_json::from_str(s).chain_err(|| "invalid json text")?;
                    return Ok(Some(msg).into());
                }

                Message::Binary(_) => return Err("binary messages not accepted".into()),

                // sending a pong is already managed by lower layers, just go to
                // the next message
                Message::Ping(_) => {}

                // Wake up ourselves to ensure the above ping logic eventually
                // sees this pong.
                Message::Pong(_) => {
                    self.pong_received = true;
                    self.pong_timeout = false;
                    task::current().notify();
                }
            }
        }
    }
}

impl<T> Sink for WebpushSocket<T>
where
    T: Sink<SinkItem = Message>,
    Error: From<T::SinkError>,
{
    type SinkItem = ServerMessage;
    type SinkError = Error;

    fn start_send(&mut self, msg: ServerMessage) -> StartSend<ServerMessage, Error> {
        if self.send_ping()?.is_not_ready() {
            return Ok(AsyncSink::NotReady(msg));
        }
        let s = serde_json::to_string(&msg).chain_err(|| "failed to serialize")?;
        match self.inner.start_send(Message::Text(s))? {
            AsyncSink::Ready => Ok(AsyncSink::Ready),
            AsyncSink::NotReady(_) => Ok(AsyncSink::NotReady(msg)),
        }
    }

    fn poll_complete(&mut self) -> Poll<(), Error> {
        try_ready!(self.send_ping());
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
        StatusCode::Ok,
        json!({
            "status": "OK",
            "version": env!("CARGO_PKG_VERSION"),
    }),
    )
}

fn write_log_check(socket: WebpushIo) -> MyFuture<()> {
    let status = StatusCode::ImATeapot;
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
        StatusCode::ImATeapot,
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
        date = time::at(time::get_time()).rfc822(),
        len = body.len(),
        body = body,
    );
    Box::new(
        tokio_io::io::write_all(socket, data.into_bytes())
            .map(|_| ())
            .chain_err(|| "failed to write status response"),
    )
}
