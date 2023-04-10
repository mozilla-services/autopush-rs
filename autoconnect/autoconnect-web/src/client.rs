use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use actix_web::web::Payload;
use actix_web::{HttpRequest, HttpResponse};
use autopush_common::util::user_agent::UserAgentInfo;
use bytes::Bytes;
use cadence::{CountedExt, StatsdClient};
use futures_util::StreamExt;
use serde_json::json;
use uuid::Uuid;

use autoconnect_common::{
    broadcast::Broadcast,
    protocol::{ClientAck, ClientMessage, ServerMessage},
};
use autoconnect_settings::AppState;
use autopush_common::db::{self, User};
use autopush_common::errors::{ApcError, ApcErrorKind, Result};
use autopush_common::notification::Notification;
use autopush_common::util::ms_since_epoch;

/// Client & Registry functions.
/// These are common functions run by connected WebSocket clients.
/// These are called from the Autoconnect server.

/// Mapping of UserAgent IDs to the Registered Client information struct
pub struct RegisteredClient {
    pub uaid: Uuid,
    pub uid: Uuid,
}
pub type ClientChannels = Arc<RwLock<HashMap<Uuid, RegisteredClient>>>;

#[allow(dead_code)]
#[derive(Clone, Default)]
struct SessionStatistics {
    /// Does this UAID require a reset
    uaid_reset: bool,
    /// Is this UAID already registered?
    existing_uaid: bool,
    /// Description of the connection type for this? (TODO: Enum?)
    connection_type: String,

    // Usage data
    /// Number of acknowledged messages that were sent directly (not vai storage)
    direct_acked: i32,
    /// number of messages sent to storage
    direct_storage: i32,
    /// number of messages taken from storage
    stored_retrieved: i32,
    /// number of message pulled from storage and acknowledged
    stored_acked: i32,
    /// number of messages total that are not acknowledged.
    nacks: i32,
    /// number of unregister requests made
    unregisters: i32,
    /// number of register requests made
    registers: i32,
}

#[allow(dead_code)]
/// The Autoconnect Client connection
pub struct Client {
    /// the UserAgent ID
    uaid: Option<Uuid>,
    /// Unique local identifier
    uid: Uuid,
    /// processing flags for this client
    flags: ClientFlags,
    /// The User Agent information block derived from the UserAgent header
    ua_info: UserAgentInfo,
    //channel: Option<SyncSender<ServerNotification>>,
    /// Handle to the database
    db: Box<dyn db::client::DbClient>,
    /// List of Channels availalbe for this UAID
    clients: ClientChannels,
    /// Metric serivce
    metrics: Arc<StatsdClient>,
    /// List of unacknowledged directly sent notifications
    unacked_direct_notifs: Vec<Notification>,
    /// List of unacknowledged notifications previously stored for transmission
    unacked_stored_notifs: Vec<Notification>,
    /// Id for the last previously unacknowledged, stored transmission
    unacked_stored_highest: Option<u64>,
    /// Timestamp for when the UA connected (used by database lookup, thus u64)
    connected_at: u64,
    /// Count of messages sent from storage (for internal metric)
    sent_from_storage: u32,
    /// Timestamp for the last Ping message
    last_ping: u64,
    /// Collected general flags and statistics for this connection.
    stats: SessionStatistics,
    /// The user information record if this registration is deferred (e.g. a
    /// previously Broadcast Only connection registers to get their first endpoint)
    deferred_user_registration: Option<User>,
    /// The router URL for this client.
    router_url: String,
}

impl Client {
    pub async fn ws_handler(req: HttpRequest, body: Payload) -> Result<HttpResponse> {
        let state = req.app_data::<AppState>().unwrap().clone();
        let client_metrics = state.metrics.clone();
        let db_client = state.db.clone();
        let clients = req.app_data::<ClientChannels>().unwrap().clone();
        let ua_string = if let Some(header) = req.headers().get(actix_web::http::header::USER_AGENT)
        {
            header
                .to_str()
                .map(|x| x.to_owned())
                .map_err(|_e| ApcErrorKind::GeneralError("Invalid user agent".to_owned()))?
        } else {
            "".to_owned()
        };
        // let (tx, rx) = mpsc::channel::<ServerNotification>(); // sync_channel(state.max_pending_notification_queue);

        // TODO: add `tx` to state list of clients

        // let heartbeat = state.auto_ping_interval.clone();
        let (response, mut session, mut msg_stream) =
            actix_ws::handle(&req, body).map_err(|e| ApcErrorKind::GeneralError(e.to_string()))?;

        let _thread = actix_rt::spawn(async move {
            //TODO: Create a new client (set values, use ..Default::default() if needed)
            let mut client = Client {
                uaid: None,
                uid: Uuid::new_v4(),
                flags: ClientFlags::default(),
                ua_info: UserAgentInfo::from(ua_string.as_str()),
                db: db_client.clone(),
                metrics: client_metrics,
                // channel: tx,
                clients: clients.clone(),
                router_url: state.router_url.clone(),
                unacked_direct_notifs: Default::default(),
                unacked_stored_notifs: Default::default(),
                unacked_stored_highest: Default::default(),
                connected_at: Default::default(),
                sent_from_storage: Default::default(),
                last_ping: Default::default(),
                stats: Default::default(),
                deferred_user_registration: Default::default(),
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
                            Err(e) => return client.process_error(session, e).await,
                            Ok(Some(resp)) => {
                                info!("<< {:?}", resp);
                                if session.text(json!(resp).to_string()).await.is_err() {
                                    session.close(None).await.unwrap();
                                    return;
                                };
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

        /*
        actix_rt::spawn(async move {
            loop {
                match rx.recv_timeout(std::time::Duration::from_secs(1)) {
                    Ok(msg) => match msg {
                        ServerNotification::Disconnect => {
                            // session2.close(Some(actix_ws::CloseCode::Normal.into()));
                            thread.abort();
                            return;
                        }
                        // Do the happy stuff.
                        _ => info!("Channel Message {:?}", msg),
                        }
                    Err(e) => {
                        dbg!("Channel Closed, exiting", e);
                        thread.abort();
                        return;
                    }
                }
            }
        });
        // */

        Ok(response)
        // TODO: Add timeout to drop if no "hello"
    }
}

impl Client {
    /// Create a new client, ensuring that we have a channel to send notifications and that the
    /// broadcast_subs are captured. Set up the local state machine if need be. This client is
    /// held by the server and links to the [autoconnect-web::client::WebPushClient]
    /// TODO:  Make sure that when this is called, the `tx` is created with
    ///   let (tx,rx) = std::sync::mpsc::channel(autoconnect_settings::options::max_pending_notification_queue);

    /// Parse and process the message calling the appropriate sub function
    async fn process_message(
        &mut self,
        msg: bytestring::ByteString,
    ) -> Result<Option<ServerMessage>> {
        // convert msg to JSON / parse
        let bytes = msg.as_bytes();
        let msg = ClientMessage::from_str(&String::from_utf8(bytes.to_vec()).map_err(|e| {
            warn!("Invalid message: {}", String::from_utf8_lossy(bytes));
            ApcErrorKind::InvalidClientMessage(e.to_string())
        })?)?;
        match msg {
            ClientMessage::Hello {
                uaid,
                channel_ids,
                use_webpush,
                broadcasts,
            } => self.hello(uaid, channel_ids, use_webpush, broadcasts).await,
            ClientMessage::Register { channel_id, key } => {
                self.register_channel(channel_id, key).await
            }
            ClientMessage::Unregister { channel_id, code } => {
                self.unregister(channel_id, code).await
            }
            ClientMessage::Ping => self.ping().await,
            ClientMessage::Ack { updates } => self.ack(updates).await,
            ClientMessage::BroadcastSubscribe { broadcasts } => self.broadcast(broadcasts).await,
            _ => Err(ApcErrorKind::GeneralError("PLACEHOLDER".to_owned()).into()),
        }
    }

    /// Handle a websocket "hello". This will return a JSON structure that contains the UAID to use.
    /// This UAID is definitative, even if it's different than the one that the client already may have
    /// provided (in rare cases, the client understands that it may need to change UAID)
    pub async fn hello(
        &mut self,
        uaid: Option<String>,
        _channel_ids: Option<Vec<Uuid>>,
        use_webpush: Option<bool>,
        _broadcasts: Option<HashMap<String, String>>,
    ) -> Result<Option<ServerMessage>> {
        self.metrics.incr_with_tags("ua.command.hello").send();
        let status = 200;

        // validate that we're using webpush
        if let Some(use_webpush) = use_webpush {
            if !use_webpush {
                return Err(ApcErrorKind::GeneralError(
                    "Client requested obsolete protocol".to_owned(),
                )
                .into());
            }
        }

        let connected_at = ms_since_epoch();

        // Check that the UAID is valid
        self.uaid = if let Some(uaid) = &uaid {
            Some(match Uuid::from_str(uaid) {
                Ok(v) => {
                    self.flags.defer_registration = false;
                    v
                }
                Err(e) => {
                    warn!("Invalid UAID specified {:?}", e);
                    self.flags.reset_uaid = true;
                    Uuid::new_v4()
                }
            })
        } else {
            // Generate a new UAID because none was provided
            self.flags.defer_registration = true;
            self.flags.reset_uaid = true;
            Some(Uuid::new_v4())
        };

        let _response = self.db.hello(
            connected_at,
            self.uaid.as_ref(),
            &self.router_url,
            self.flags.defer_registration,
        );

        if let Some(uaid) = self.uaid {
            // store the channel to use to talk to us.
            self.clients.write().unwrap().insert(
                uaid,
                RegisteredClient {
                    uaid,
                    uid: uuid::Uuid::new_v4(),
                    // tx: self.channel.clone()
                },
            );
            // update the user record with latest connection data (or fill in a new record with the data we get)
            let mut user_record = self.db.get_user(&uaid).await?.unwrap_or_default();
            user_record.uaid = uaid;
            user_record.connected_at = ms_since_epoch();
            user_record.last_connect = Some(ms_since_epoch());

            // Don't record the anonymous user if we're deferring registration
            if !self.flags.defer_registration {
                self.db.update_user(&user_record).await?;
            }
        }

        // TODO: record the broadcasts and check that those are valid

        // return response.

        // Convert this to `HashMap<std::string::String, protocol::BroadcastValue>`
        // let broadcasts = ...;
        Ok(Some(ServerMessage::Hello {
            uaid: self.uaid.unwrap().as_simple().to_string(),
            status,
            use_webpush,
            broadcasts: HashMap::new(),
        }))
    }

    /// Add the provided `channel_id` to the list of associated channels for this UAID, generate a new endpoint using the key
    pub async fn register_channel(
        &mut self,
        channel_id_string: String,
        _key: Option<String>,
    ) -> Result<Option<ServerMessage>> {
        debug!("Got a register command"; "uaid"=>self.uaid.map(|v| v.to_string()), "channel_id" => &channel_id_string );

        let channel_id = Uuid::parse_str(&channel_id_string).map_err(|_e| {
            ApcErrorKind::InvalidClientMessage(format!("Invalid ChannelID: {channel_id_string}"))
        })?;
        if channel_id.as_hyphenated().to_string() != channel_id_string {
            return Err(ApcErrorKind::InvalidClientMessage(format!(
                "Invalid UUID format, not lower-case/dashed: {channel_id_string}"
            ))
            .into());
        }

        let _uaid = self.uaid;

        let status = 200;

        let push_endpoint = "REPLACE ME".to_owned();

        Ok(Some(ServerMessage::Register {
            channel_id,
            status,
            push_endpoint,
        }))
    }

    /// Drop the provided `channel_id` from the list of associated channels for this UAID
    pub async fn unregister(
        &mut self,
        channel_id: Uuid,
        _key: Option<u32>,
    ) -> Result<Option<ServerMessage>> {
        let status = 200;

        let _push_endpoint = "REPLACE ME".to_owned();

        Ok(Some(ServerMessage::Unregister { channel_id, status }))
    }

    /// Return a Ping / Set of broadcast updates
    pub async fn ping(&mut self) -> Result<Option<ServerMessage>> {
        Ok(None)
    }

    /// Handle the list of Acknowledged messages, removing them from retransmission.
    pub async fn ack(&mut self, _updates: Vec<ClientAck>) -> Result<Option<ServerMessage>> {
        Ok(None)
    }

    /// Add new set of IDs to monitor for broadcast changes
    pub async fn broadcast(
        &mut self,
        _broadcasts: HashMap<String, String>,
    ) -> Result<Option<ServerMessage>> {
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
        None
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
/*
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
*/

/// Set of Session specific flags for this WebPushClient
pub struct ClientFlags {
    /// Defer registration
    defer_registration: bool,
    /// Whether check_storage queries for topic (not "timestamped") messages
    _include_topic: bool,
    /// Flags the need to increment the last read for timestamp for timestamped messages
    _increment_storage: bool,
    /// Whether this client needs to check storage for messages
    _check: bool,
    /// Flags the need to drop the user record
    reset_uaid: bool,
    _rotate_message_table: bool,
}

// use ClientFlags::default() not ::new()
impl Default for ClientFlags {
    fn default() -> Self {
        Self {
            defer_registration: false,
            _include_topic: true,
            _increment_storage: false,
            _check: false,
            reset_uaid: false,
            _rotate_message_table: false,
        }
    }
}

/*
/// An Unauthorized client, which is the initial state of any websocket connection.
/// The client must properly identify within the initial timeout period, or else the
/// connection is rest.
pub struct UnAuthClientData<T> {
    // srv: Rc<Server>,
    ws: T,
    user_agent: String,
    broadcast_subs: Rc<RefCell<BroadcastSubs>>,
}

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
                if let Some(_client) = clients.insert(client.uaid, client) {
                    /*
                    if client.tx.send(ServerNotification::Disconnect).is_ok() {
                        debug!("Told client to disconnect as a new one wants to connect");
                    }
                    // */
                };
            })
            .map_err(|_e| ApcErrorKind::GeneralError("Lock Poisoned".to_owned()))?;
        Ok(())
    }

    /// A notification has come for the UAID
    pub async fn notify(&self, uaid: Uuid, _notif: Notification) -> Result<()> {
        self.clients
            .read()
            .map(|clients| {
                debug!("Sending notification");
                if let Some(_client) = clients.get(&uaid) {
                    debug!("Found a client to deliver a notification to");
                    /*
                    return if client
                        .tx
                        .send(ServerNotification::Notification(notif))
                        .is_ok()
                    {
                        debug!("Dropped notification in queue");
                        Ok(())
                    } else {
                        Err(ApcErrorKind::GeneralError("Could not send notification".to_owned()).into())
                    }
                    // */
                    Ok(())
                } else {
                    Ok(())
                }
            })
            .map_err(|_e| ApcErrorKind::GeneralError("Lock Poisoned".to_owned()))?
    }

    /// A check for notification command has come for the uaid
    pub async fn check_storage(&self, _uaid: Uuid) -> Result<()> {
        self.clients
            .read()
            .map(|_clients| {
                /*
                if let Some(client) = clients.get(&uaid) {
                    if client.tx.send(ServerNotification::CheckStorage).is_ok() {
                        debug!("Told client to check storage");
                        return Ok(());
                    }
                }
                // */
                Err(ApcErrorKind::GeneralError("Could not store notification".to_owned()).into())
            })
            .map_err(|_e| ApcErrorKind::GeneralError("Lock Poisoned".to_owned()))?
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
            .map_err(|_e| ApcErrorKind::GeneralError("Lock Poisoned".to_owned()))?
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

// TODO: remove dead code
#[allow(dead_code)]
impl NotifManager {
    pub async fn on_push(
        state: &AppState,
        uaid: Uuid,
        notification: Notification,
    ) -> Result<HttpResponse> {
        state.clients.notify(uaid, notification).await?;
        Ok(HttpResponse::Ok().finish())
    }

    pub async fn on_notif(state: &AppState, uaid: Uuid) -> Result<HttpResponse> {
        if state.clients.check_storage(uaid).await.is_ok() {
            return Ok(HttpResponse::Ok().finish());
        };
        let body = Bytes::from_static(b"Client not available");
        Ok(HttpResponse::NotFound().body::<Bytes>(body))
    }
}
