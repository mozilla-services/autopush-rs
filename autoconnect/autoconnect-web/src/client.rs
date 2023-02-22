use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::str::FromStr;
use std::sync::mpsc;

use actix_web::dev::ServiceRequest;
use actix_web::web::{Data, Payload};
use actix_web::{HttpRequest, HttpResponse};
use bytes::Bytes;
use cadence::{StatsdClient, CountedExt};
use futures_locks::RwLock;
use futures_util::{FutureExt, StreamExt};
use serde_json::json;
use uuid::Uuid;

use autoconnect_registry::RegisteredClient;
use autoconnect_settings::options::ServerOptions;
use autoconnect_ws::ServerNotification;
use autopush_common::db::UserRecord;
use autopush_common::errors::{ApcError, ApcErrorKind, Result};
use autopush_common::notification::Notification;
use autopush_common::util::ms_since_epoch;

use crate::broadcast::{Broadcast, BroadcastSubs};
use crate::protocol::{ClientMessage, ServerMessage, ClientAck};

/// Client & Registry functions.
/// These are common functions run by connected WebSocket clients.
/// These are called from the Autoconnect server.

/// The Autoconnect Client connection
#[derive(Debug)]
pub struct Client {
    uaid: Option<Uuid>,
    metrics: Arc<StatsdClient>,
}

impl Default for Client {
    fn default() -> Client {
        Client {
        uaid: None,
        metrics: Arc::new(StatsdClient::builder("autoconnect", cadence::NopMetricSink).build()),
        }
    }
}

impl Client {
    pub async fn ws_handler(req: HttpRequest, body: Payload) -> Result<HttpResponse> {
        let state = req.app_data::<ServerOptions>().unwrap();
        let client_metrics = state.metrics.clone();
        let (tx, rx) = mpsc::sync_channel(state.max_pending_notification_queue);

        // TODO: add `tx` to state list of clients

        let heartbeat = state.auto_ping_interval.clone();
        let (response, mut session, mut msg_stream) =
            actix_ws::handle(&req, body).map_err(|e| ApcErrorKind::GeneralError(e.to_string()))?;

        let thread = actix_rt::spawn(async move {
            //TODO: Create a new client (set values, use ..Default::default() if needed)
            let mut client = Client{
                metrics: client_metrics.clone(),
                ..Default::default()
            };
            while let Some(msg) = msg_stream.next().await {
                match msg {
                    Ok(actix_ws::Message::Ping(msg)) => {
                        // TODO: Add megaphone handling
                        session.pong(&msg).await.unwrap();
                    }
                    Ok(actix_ws::Message::Text(msg)) => {
                        info!(">> {:?}", msg);
                        match client.process_message(msg).await {
                            Err(e) => {
                                return client.process_error(session, e).await
                            }
                            Ok(Some(resp)) => {
                                info!("<< {:?}", resp);
                                session.text(json!(resp).to_string());
                            }
                            Ok(None) => {}
                        };
                    }
                    _ => {
                        error!("Unsupported socket message: {:?}", msg);
                        let _ = session.close(None).await;
                        return;
                    }
                }
            }
        });

        actix_rt::spawn(async move {
            loop {
                if let Ok(msg) = rx.recv_timeout(heartbeat) {
                    match msg {
                        ServerNotification::Disconnect => {
                            // session2.close(Some(actix_ws::CloseCode::Normal.into()));
                            thread.abort();
                        }
                        // Do the happy stuff.
                        _ => info!("Channel Message {:?}", msg),
                    }
                }
            }
        });

        Ok(response)
        // TODO: Add timeout to drop if no "hello"
    }
}

impl Client {
    /// Create a new client, ensuring that we have a channel to send notifications and that the
    /// broadcast_subs are captured. Set up the local state machine if need be. This client is
    /// held by the server and links to the [autoconnect-web::client::WebPushClient]
    /// TODO:
    /// Make sure that when this is called, the `tx` is created with
    /// ```
    ///   let (tx,rx) = std::sync::mpsc::channel(autoconnect_settings::options::max_pending_notification_queue);
    /// ```

    /// Parse and process the message calling the appropriate sub function
    async fn process_message(&mut self, msg: bytestring::ByteString) -> Result<Option<ServerMessage>> {
        // convert msg to JSON / parse
        let bytes = msg.as_bytes();
        let msg = ClientMessage::from_str(&String::from_utf8(bytes.to_vec()).map_err(|e| {
            warn!("Invalid message: {}", String::from_utf8_lossy(bytes));
            ApcErrorKind::InvalidClientMessage(e.to_string())
        })?)?;
        return match msg {
                ClientMessage::Hello{uaid,channel_ids, use_webpush, broadcasts} => self.hello(uaid, channel_ids, use_webpush, broadcasts).await,
                ClientMessage::Register{channel_id, key} => self.register(channel_id, key).await,
                ClientMessage::Unregister{channel_id, code} => self.unregister(channel_id, code).await,
                ClientMessage::Ping => self.ping().await,
                ClientMessage::Ack{updates} => self.ack(updates).await,
                ClientMessage::BroadcastSubscribe{broadcasts} => self.broadcast(broadcasts).await,
                _ => Err(ApcErrorKind::GeneralError("PLACEHOLDER".to_owned()).into()),
            }
        }

    /// Handle a websocket "hello". This will return a JSON structure that contains the UAID to use.
    /// This UAID is definitative, even if it's different than the one that the client already may have
    /// provided (in rare cases, the client understands that it may need to change UAID)
    pub async fn hello(&mut self, uaid: Option<String>, channel_ids: Option<Vec<Uuid>>, use_webpush: Option<bool>, broadcasts: Option<HashMap<String, String>>) -> Result<Option<ServerMessage>> {

        self.metrics.incr_with_tags("hello").send();

        let new_uaid = Uuid::from_str(&uaid.unwrap_or_default()).unwrap_or_else(|_| Uuid::new_v4());
        self.uaid = Some(new_uaid.clone());
        let status = 200;
        // Check that the UAID is valid
        // validate that we're using webpush
        // record the broadcasts and that those are valid
        // return response.

        // Convert this to `HashMap<std::string::String, protocol::BroadcastValue>`
        // let broadcasts = ...;
        Ok(Some(ServerMessage::Hello {
            uaid: new_uaid.as_simple().to_string(),
            status,
            use_webpush: use_webpush,
            broadcasts: HashMap::new(),
        }))
    }

    /// Add the provided `channel_id` to the list of associated channels for this UAID, generate a new endpoint using the key
    pub async fn register(&mut self, channel_id:String, key:Option<String>) -> Result<Option<ServerMessage>> {
        let status = 200;

        let channel_id = Uuid::from_str(&channel_id)?;

        let push_endpoint = "REPLACE ME".to_owned();

        Ok(Some(ServerMessage::Register { channel_id, status: 200, push_endpoint}))

    }

    /// Drop the provided `channel_id` from the list of associated channels for this UAID
    pub async fn unregister(&mut self, channel_id:Uuid, key:Option<u32>) -> Result<Option<ServerMessage>> {
        let status = 200;

        let push_endpoint = "REPLACE ME".to_owned();

        Ok(Some(ServerMessage::Unregister { channel_id, status: 200}))
    }

    /// Return a Ping / Set of broadcast updates
    pub async fn ping(&mut self) -> Result<Option<ServerMessage>> {
        Ok(None)
    }

    /// Handle the list of Acknowledged messages, removing them from retransmission.
    pub async fn ack(&mut self, updates: Vec<ClientAck>) -> Result<Option<ServerMessage>> {
        Ok(None)
    }

    /// Add new set of IDs to monitor for broadcast changes
    pub async fn broadcast(&mut self, broadcasts: HashMap<String, String>) -> Result<Option<ServerMessage>> {
        Ok(None)
    }

    /// Process the error, logging it and terminating the connection.
    pub async fn process_error(&mut self, session: actix_ws::Session, e: ApcError) {
        // send error to sentry if appropriate
        error!("Error:: {e:?}");
        // For now, close down normally and eat the close error.
        session
            .close(Some(actix_ws::CloseCode::Normal.into()))
            .await
            .unwrap_or_default();
    }

    /// Get the list of broadcasts that have changed recently.
    pub async fn broadcast_delta(&mut self) -> Option<Vec<Broadcast>> {
        // self.srv.broadcast_delta(&mut self.broadcast_subs.borrow_mut())
        return None;
    }

    pub fn shutdown(&mut self) -> Result<()> {
        // Shutdown should be done on the server side.
        /*
        if let Err(e) = self.tx.send(ServerNotification::Disconnect) {
            debug!("Failure to shut down client: {e:?}")
        };
        */
        Ok(())
    }
}
/// Websocket session statistics
#[derive(Clone, Default)]
struct SessionStatistics {
    // User data
    /// The User Agent Identifier
    uaid: String,
    /// Should the UAID be reset because it's invalid?
    uaid_reset: bool,
    /// Has this UAID already registered?
    existing_uaid: bool,
    /// This value is almost always "webpush" for desktop connections
    connection_type: String,

    // Usage data
    /// Number of messages that the client successfully received.
    direct_acked: i32,
    /// Number of messages that were not successfully received, so they were stored
    direct_storage: i32,
    /// Number of messages taken from storage
    stored_retrieved: i32,
    /// Number of messages taken from storage, and successfully received.
    stored_acked: i32,
    /// Number of messages not accepted
    nacks: i32,
    /// Number of channels that were closed or removed
    unregisters: i32,
    /// Number of channels created
    registers: i32,
}

/// Represent the state for a valid WebPush client that is authenticated
pub struct WebPushClient {
    /// User Agent ID
    uaid: Uuid,
    /// Unique local Identifier
    uid: Uuid,
    /// Incoming Notification channel
    rx: mpsc::Receiver<Notification>,
    /// Various state flags
    flags: ClientFlags,
    /// Semi-obsolete message month (used for rotation)
    message_month: String,
    /// List of unacknowledged Notifications that are delivered directly
    unacked_direct_notifs: Vec<Notification>,
    /// List of unacknowledged Notifications that are from db storage
    unacked_stored_notifs: Vec<Notification>,
    // Highest version from stored, retained for use with increment
    // when all the unacked storeds are ack'd
    unacked_stored_highest: Option<u64>,
    /// When the WebpushClient connected
    connected_at: u64,
    /// Total number of notification sent from db storage so far
    sent_from_storage: u32,
    /// Time of the last Ping (UTC)
    last_ping: u64,
    /// Various statistics around this session
    stats: SessionStatistics,
    /// The User Data (if the registration has been deferred)
    deferred_user_registration: Option<UserRecord>,
}

impl WebPushClient {
    pub fn new(uaid: Uuid, rx: mpsc::Receiver<Notification>) -> Self {
        Self {
            uaid,
            uid: Uuid::new_v4(),
            rx,
            flags: Default::default(),
            message_month: Default::default(),
            unacked_direct_notifs: Default::default(),
            unacked_stored_notifs: Default::default(),
            unacked_stored_highest: Default::default(),
            connected_at: ms_since_epoch(),
            sent_from_storage: Default::default(),
            last_ping: Default::default(),
            stats: Default::default(),
            deferred_user_registration: Default::default(),
        }
    }
}

impl WebPushClient {
    /// Are there any pending notifications that have not yet been acknowledged by the client?
    fn unacked_messages(&self) -> bool {
        !self.unacked_stored_notifs.is_empty() || !self.unacked_direct_notifs.is_empty()
    }
}

/// Set of Session specific flags for this WebPushClient
pub struct ClientFlags {
    /// Whether check_storage queries for topic (not "timestamped") messages
    include_topic: bool,
    /// Flags the need to increment the last read for timestamp for timestamped messages
    increment_storage: bool,
    /// Whether this client needs to check storage for messages
    check: bool,
    /// Flags the need to drop the user record
    reset_uaid: bool,
    rotate_message_table: bool,
}

// use ClientFlags::default() not ::new()
impl Default for ClientFlags {
    fn default() -> Self {
        Self {
            include_topic: true,
            increment_storage: false,
            check: false,
            reset_uaid: false,
            rotate_message_table: false,
        }
    }
}

/// An Unauthorized client, which is the initial state of any websocket connection.
/// The client must properly identify within the initial timeout period, or else the
/// connection is rest.
pub struct UnAuthClientData<T> {
    // srv: Rc<Server>,
    ws: T,
    user_agent: String,
    broadcast_subs: Rc<RefCell<BroadcastSubs>>,
}

/*
impl<T> UnAuthClientData <T>
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
*/

/*
pub struct AuthClientData<T> {
    srv: Rc<Server>,
    ws: T,
    webpush: Rc<RefCell<WebPushClient>>,
    broadcast_subs: Rc<RefCell<BroadcastSubs>>,
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
*/

/* the rest of the original client.rs contains the Client state machine functions.
these should probably go into an impl inside of autoconnect-ws-clientsm  */

/* registry.rs? ⬇ */
/// mapping of UAIDs to connected clients.
#[derive(Default)]
pub struct ClientRegistry {
    clients: RwLock<HashMap<Uuid, RegisteredClient>>,
}

impl ClientRegistry {
    pub async fn connect(&self, client: RegisteredClient) -> Result<()> {
        debug!("Connecting a client!");
        self.clients
            .write()
            .map(|mut clients| {
                if let Some(client) = clients.insert(client.uaid, client) {
                    if client.tx.send(ServerNotification::Disconnect).is_ok() {
                        debug!("Told client to disconnect as a new one wants to connect");
                    }
                };
            })
            .await;
        Ok(())
    }

    /// A notification has come for the UAID
    pub async fn notify(&self, uaid: Uuid, notif: Notification) -> Result<()> {
        self.clients
            .read()
            .map(|clients| {
                debug!("Sending notification");
                if let Some(client) = clients.get(&uaid) {
                    debug!("Found a client to deliver a notification to");
                    if client
                        .tx
                        .send(ServerNotification::Notification(notif))
                        .is_ok()
                    {
                        debug!("Dropped notification in queue");
                        return Ok(());
                    }
                }
                Err(ApcErrorKind::GeneralError("Could not send notification".to_owned()).into())
            })
            .await
    }

    /// A check for notification command has come for the uaid
    pub async fn check_storage(&self, uaid: Uuid) -> Result<()> {
        self.clients
            .read()
            .map(|clients| {
                if let Some(client) = clients.get(&uaid) {
                    if client.tx.send(ServerNotification::CheckStorage).is_ok() {
                        debug!("Told client to check storage");
                        return Ok(());
                    }
                }
                Err(ApcErrorKind::GeneralError("Could not store notification".to_owned()).into())
            })
            .await
    }

    /// The client specified by `uaid` has disconnected.
    #[allow(clippy::clone_on_copy)]
    pub async fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> Result<()> {
        debug!("Disconnecting client!");
        let uaidc = uaid.clone();
        let uidc = uid.clone();
        self.clients
            .write()
            .map(|mut clients| {
                let client_exists = clients
                    .get(&uaidc)
                    .map_or(false, |client| client.uid == uidc);
                if client_exists {
                    clients.remove(&uaidc).expect("Couldn't remove client?");
                    return Ok(());
                }
                Err(ApcErrorKind::GeneralError("Could not remove client".to_owned()).into())
            })
            .await
    }
}

/// Container for client actions
///
/// These functions will be called by the state
struct ClientActions {
    //srv: Rc<Server>,
    uaid: Option<Uuid>,
}

impl ClientActions {
    pub async fn on_hello(&mut self, req: &ServiceRequest) -> Result<()> {
        let data = req.app_data::<Data<ServerOptions>>().unwrap();
        let _connected_at = ms_since_epoch();
        trace!("### AwaitHello UAID: {:?}", self.uaid);
        // Defer registration (don't write the user to the router table yet)
        // when no uaid was specified. We'll get back a pending DynamoDbUser
        // from the HelloResponse. It'll be potentially written to the db later
        // whenever the user first subscribes to a channel_id
        // (ClientMessage::Register).
        let _defer_registration = self.uaid.is_none();
        /*
            // roll those functions into here using normalized db_client calls?
            let response = data.db_client.hello(
                connected_at,
                self.uaid,
                data.router_url,
                defer_registration
            );
        */
        // lookup_user
        if let Some(uaid) = self.uaid {
            if let Some(_user) = data.db_client.get_user(&uaid).await? {
                // handle_user_result
            }
        }
        Ok(())
    }
}

/// Handle incoming routed notifications from autoendpoint servers.
///
/// These requests include the notification in the body of the request.
/// PUT /push/{uaid} - send notification to the connected client
///    return OK{}, NOT_FOUND{"Client not available."}, BAD_REQUEST{"Unable to decode payload"}
/// PUT /notif/{method_name}/{uaid} - check if uaid is in storage
///    return OK{}, NOT_FOUND{"Client not available."}, OK(result of `check_storage`)
///
#[derive(Clone, Debug, Default)]
struct NotifManager {}

impl NotifManager {
    pub async fn on_push(
        state: &ServerOptions,
        uaid: Uuid,
        notification: Notification,
    ) -> Result<HttpResponse> {
        let _r = state.registry.notify(uaid, notification).await?;
        Ok(HttpResponse::Ok().finish())
    }

    pub async fn on_notif(state: &ServerOptions, uaid: Uuid) -> Result<HttpResponse> {
        if state.registry.check_storage(uaid).await.is_ok() {
            return Ok(HttpResponse::Ok().finish());
        };
        let body = Bytes::from_static(b"Client not available");
        Ok(HttpResponse::NotFound().body::<Bytes>(body.into()))
    }
}
