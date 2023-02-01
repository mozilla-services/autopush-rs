use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use actix_web::dev::ServiceRequest;
use actix_web::web::Data;
use autopush_common::db::UserRecord;
use autopush_common::errors::{ApcErrorKind, Result};
use autopush_common::notification::Notification;
use autopush_common::util::ms_since_epoch;
use futures_locks::RwLock;
use futures_util::FutureExt;
use uuid::Uuid;

use autoconnect_settings::options::ServerOptions;

use crate::broadcast::{Broadcast, BroadcastSubs};

/// Client & Registry functions.
/// These are common functions run by connected WebSocket clients.
/// These are called from the Autoconnect server.

/// A connected Websocket client.
pub struct RegisteredClient {
    /// The user agent's unique ID.
    pub uaid: Uuid,
    /// The local ID, used to potentially distinquish multiple UAID connections.
    pub uid: Uuid,
    // / The channel for delivery of incoming notifications.
    //pub tx: mpsc::UnboundedSender<ServerNotification>,
}

/// The Autoconnect Client connection
#[derive(Default)]
pub struct Client /*<T>
where
    T: Stream<Item = ClientMessage, Error = Error>
        + Sink<SinkItem = ServerMessage, SinkError = Error>
        + 'static,
    */ {
    // state_machine: UnAuthClientStateFuture<T>, // Client
    // / Pointer to the server structure for this client
    // srv: Rc<Server>,
    // / List of subscribed broadcasts
    // broadcast_subs: Rc<RefCell<BroadcastSubs>>,
    // / Channel for incoming notifications
    //tx: mpsc::UnboundedSender<ServerNotification>,
}

impl Client {
    /// Create a new client, ensuring that we have a channel to send notifications and that the
    /// broadcast_subs are captured. Set up the local state machine if need be.
    pub async fn new() -> Client {
        Self::default()
    }

    /// Get the list of broadcasts that have changed recently.
    pub async fn broadcast_delta(&mut self) -> Option<Vec<Broadcast>> {
        // self.srv.broadcast_delta(&mut self.broadcast_subs.borrow_mut())
        return None;
    }

    pub fn shutdown(&mut self) {
        //self.tx.unbounded_send(ServerNotification::Disconnect);
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
    uaid: Uuid,
    uid: Uuid,
    // rx: mpsc::UnboundedReceiver<ServerNotification>,
    flags: ClientFlags,
    message_month: String,
    unacked_direct_notifs: Vec<Notification>,
    unacked_stored_notifs: Vec<Notification>,
    // Highest version from stored, retained for use with increment
    // when all the unacked storeds are ack'd
    unacked_stored_highest: Option<u64>,
    connected_at: u64,
    sent_from_storage: u32,
    last_ping: u64,
    stats: SessionStatistics,
    deferred_user_registration: Option<UserRecord>,
}

impl Default for WebPushClient {
    fn default() -> Self {
        // let (_, rx) = mpsc::unbounded();
        Self {
            uaid: Default::default(),
            uid: Default::default(),
            // rx,
            flags: Default::default(),
            message_month: Default::default(),
            unacked_direct_notifs: Default::default(),
            unacked_stored_notifs: Default::default(),
            unacked_stored_highest: Default::default(),
            connected_at: Default::default(),
            sent_from_storage: Default::default(),
            last_ping: Default::default(),
            stats: Default::default(),
            deferred_user_registration: Default::default(),
        }
    }
}

impl WebPushClient {
    fn unacked_messages(&self) -> bool {
        !self.unacked_stored_notifs.is_empty() || !self.unacked_direct_notifs.is_empty()
    }
}

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
                if let Some(_client) = clients.insert(client.uaid, client) {
                    // Drop existing connection
                    // if client.tx.unbounded_send(ServerNotification::Disconnect).is_ok(){
                    debug!("Told client to disconnect as a new one wants to connect");
                };
            })
            .await;
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
                    let result = client
                        .tx
                        .unbounded_send(ServerNotification::Notification(notif));
                    if result.is_ok() {
                        debug!("Dropped notification in queue");
                        return Ok(());
                    }
                    */
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
                if let Some(_client) = clients.get(&uaid) {
                    /*
                    let result = client.tx.unbounded_send(ServerNotification::CheckStorage);
                    if result.is_ok() {
                        debug!("Told client to check storage");
                        return Ok(());
                    }
                    */
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
/// PUT /push/{method_name}/{uaid} - send notification to the connected client
///    return OK{}, NOT_FOUND{"Client not available."}, BAD_REQUEST{"Unable to decode payload"}
/// PUT /notif/{method_name}/{uaid} - check if uaid is in storage
///    return OK{}, NOT_FOUND{"Client not available."}, OK(result of `check_storage`)
///
struct NotifManager {}

impl NotifManager {}
