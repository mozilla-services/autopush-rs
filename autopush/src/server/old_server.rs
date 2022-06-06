//! [ed] Old websocket server code.
//!
//! Since this pre-dates actix, there's a lot of boilerplate
//! that's no longer needed (e.g. building the TLS connection)
//! The message handler bits are dealt with in the `Server::new`
//! loop handler, and those will need to be ported over to aserver.
//! Broadcast functions should probably be extricated and put into
//! a specific broadcast module.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::net::SocketAddr;
use std::panic;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;

use cadence::StatsdClient;
use futures::channel::oneshot;
use hyper::{server::conn::Http, StatusCode};
use openssl::ssl::SslAcceptor;
use tokio::time::timeout;
use tokio_core::net::TcpListener;
use tokio_core::reactor::{Core, Handle};
use tokio_tungstenite::accept_hdr_async;
use tungstenite::handshake::server::Request;

use autopush_common::db::dynamodb::DynamoStorage;
// use autopush_common::db::postgres::PostgresStorage;
use autopush_common::db::postgres::PgClientImpl;
use autopush_common::db::DbCommandClient;
use autopush_common::errors::{ApiError, ApiErrorKind};
use autopush_common::logging;

use crate::http;
use crate::megaphone::{Broadcast, BroadcastChangeTracker, BroadcastSubs, BroadcastSubsInit};
use crate::server::dispatch::{Dispatch, RequestType};
use crate::server::metrics::metrics_from_opts;
use crate::server::protocol::BroadcastValue;
use crate::server::registry::ClientRegistry;

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
                    result = Err(From::from(ApiErrorKind::Thread(err)));
                }
            }
        }
        logging::reset_logging();
        result
    }
}

pub struct Server {
    pub clients: Arc<ClientRegistry>,
    broadcaster: RefCell<BroadcastChangeTracker>,
    pub db_client: Box<dyn DbCommandClient>,
    open_connections: Cell<u32>,
    tls_acceptor: Option<SslAcceptor>,
    pub opts: Arc<ServerOptions>,
    pub handle: Handle,
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
    async fn new(opts: &Arc<ServerOptions>) -> Result<(Rc<Server>, Core)> {
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

        let db_client = Box::new(if opts.db_dsn.is_empty() {
            DynamoStorage::from_opts(
                &opts._message_table_name,
                &opts._router_table_name,
                metrics.clone(),
            )
            .compat()?
        } else {
            PgClientImpl::from_opts(
                &opts.db_dsn,
                &opts.message_table,
                &opts.router_table,
                opts.meta_table,
                metrics.clone(),
            )?
        });

        let srv = Rc::new(Server {
            opts: opts.clone(),
            broadcaster: RefCell::new(broadcaster),
            db_client,
            clients: Arc::new(ClientRegistry::default()),
            open_connections: Cell::new(0),
            handle: core.handle(),
            tls_acceptor: tls::configure(opts),
            metrics,
        });
        let addr = SocketAddr::from(([0, 0, 0, 0], srv.opts.port));
        let ws_listener = TcpListener::bind(&addr, &srv.handle)?;

        let handle = core.handle();
        let srv2 = srv.clone();
        let ws_srv =
            ws_listener
                .incoming()
                .map_err(ApiError::from)
                .for_each(|(socket, addr)| async move {
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
                    let socket = tls::accept(&srv, socket).await?;

                    // Figure out if this is a websocket or a `/status` request,
                    let request = Dispatch::new;

                    // Time out both the TLS accept (if any) along with the dispatch
                    // to figure out where we're going.
                    let request =
                        timeout(srv.opts.open_handshake_timeout.unwrap_or_default(), request);
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

                    let client = request.await?.map(|(socket, request)| async move {
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
                                let ws = accept_hdr_async(socket, callback)
                                    .map_err(|| "failed to accept client");
                                let ws = timeout(srv2.opts.open_handshake_timeout, ws);

                                // Once the handshake is done we'll start the main
                                // communication with the client, managing pings
                                // here and deferring to `Client` to start driving
                                // the internal state machine.
                                Box::new(
                                    ws.await?
                                        .map(|ws| {
                                            PingManager::new(&srv2, ws, uarx)
                                                .map_err(|| "failed to make ping handler")?
                                        })
                                        .flatten(),
                                )
                            }
                        }
                    });

                    let srv = srv.clone();
                    handle.spawn(client.await?.map(move |res| {
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
            )
            .expect("Unable to start megaphone updater");
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

    /// Initialize broadcasts for a newly connected client
    pub fn broadcast_init(
        &self,
        desired_broadcasts: &[Broadcast],
    ) -> (BroadcastSubs, HashMap<String, BroadcastValue>) {
        debug!("Initialized broadcasts");
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
