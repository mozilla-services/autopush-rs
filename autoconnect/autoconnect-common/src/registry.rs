use std::collections::HashMap;

use futures::channel::mpsc;
use futures_locks::RwLock;
use uuid::Uuid;

use autopush_common::errors::{ApcErrorKind, Result};
use autopush_common::notification::Notification;

use crate::protocol::ServerNotification;

/// A connected Websocket client.
#[derive(Debug)]
pub struct RegisteredClient {
    /// The user agent's unique ID.
    pub uaid: Uuid,
    /// The local ID, used to potentially distinquish multiple UAID connections.
    pub uid: Uuid,
    /// The inbound channel for delivery of locally routed Push Notifications
    pub tx: mpsc::UnboundedSender<ServerNotification>,
}

/// Contains a mapping of UAID to the associated RegisteredClient.
#[derive(Default)]
pub struct ClientRegistry {
    clients: RwLock<HashMap<Uuid, RegisteredClient>>,
}

impl ClientRegistry {
    /// Informs this server that a new `client` has connected
    ///
    /// For now just registers internal state by keeping track of the `client`,
    /// namely its channel to send notifications back.
    pub async fn connect(
        &self,
        uaid: Uuid,
        uid: Uuid,
        tx: mpsc::UnboundedSender<ServerNotification>,
    ) {
        debug!("Connecting a client!");
        let client = RegisteredClient { uaid, uid, tx };
        let mut clients = self.clients.write().await;
        if let Some(client) = clients.insert(client.uaid, client) {
            // Drop existing connection
            let result = client.tx.unbounded_send(ServerNotification::Disconnect);
            if result.is_ok() {
                debug!("Told client to disconnect as a new one wants to connect");
            }
        }
    }

    /// A notification has come for the uaid
    pub async fn notify(&self, uaid: Uuid, notif: Notification) -> Result<()> {
        let clients = self.clients.read().await;
        debug!("Sending notification");
        if let Some(client) = clients.get(&uaid) {
            debug!("Found a client to deliver a notification to");
            let result = client
                .tx
                .unbounded_send(ServerNotification::Notification(notif));
            if result.is_ok() {
                debug!("Dropped notification in queue");
                return Ok(());
            }
        }
        Err(ApcErrorKind::GeneralError("User not connected".into()).into())
    }

    /// A check for notification command has come for the uaid
    pub async fn check_storage(&self, uaid: Uuid) -> Result<()> {
        let clients = self.clients.read().await;
        if let Some(client) = clients.get(&uaid) {
            let result = client.tx.unbounded_send(ServerNotification::CheckStorage);
            if result.is_ok() {
                debug!("Told client to check storage");
                return Ok(());
            }
        }
        Err(ApcErrorKind::GeneralError("User not connected".into()).into())
    }

    /// The client specified by `uaid` has disconnected.
    pub async fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> Result<()> {
        debug!("Disconnecting client!");
        let mut clients = self.clients.write().await;
        let client_exists = clients.get(uaid).map_or(false, |client| client.uid == *uid);
        if client_exists {
            clients.remove(uaid).expect("Couldn't remove client?");
            return Ok(());
        }
        Err(ApcErrorKind::GeneralError("User not connected".into()).into())
    }
}
