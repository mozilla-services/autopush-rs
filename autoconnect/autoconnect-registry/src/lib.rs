// move to own crate to avoid cyclic crate errors.
#[macro_use]
extern crate slog_scope;

use std::collections::HashMap;
use std::sync::mpsc;

use futures_locks::RwLock;
use uuid::Uuid;

use autoconnect_ws::ServerNotification;
use autopush_common::errors::{ApcErrorKind, Result};
use autopush_common::notification::Notification;

/// A connected Websocket client.
pub struct RegisteredClient {
    /// The user agent's unique ID.
    pub uaid: Uuid,
    /// The local ID, used to potentially distinquish multiple UAID connections.
    pub uid: Uuid,
    /// The channel for delivery of incoming notifications.
    pub tx: mpsc::Sender<ServerNotification>,
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
    pub async fn connect(&self, client: RegisteredClient) -> Result<()> {
        debug!("Connecting a client!");
        let mut clients = self.clients.write().await;
        if let Some(client) = clients.insert(client.uaid, client) {
            // Drop existing connection
            let result = client.tx.send(ServerNotification::Disconnect);
            if result.is_ok() {
                debug!("Told client to disconnect as a new one wants to connect");
            }
        }
        Ok(())
    }

    /// A notification has come for the uaid
    pub async fn notify(&self, uaid: Uuid, notif: Notification) -> Result<()> {
        let clients = self.clients.read().await;
        debug!("Sending notification");
        if let Some(client) = clients.get(&uaid) {
            debug!("Found a client to deliver a notification to");
            let result = client.tx.send(ServerNotification::Notification(notif));
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
            let result = client.tx.send(ServerNotification::CheckStorage);
            if result.is_ok() {
                debug!("Told client to check storage");
                return Ok(());
            }
        }
        Err(ApcErrorKind::GeneralError("User not connected".into()).into())
    }

    /// The client specified by `uaid` has disconnected.
    #[allow(clippy::clone_on_copy)]
    pub async fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> Result<()> {
        debug!("Disconnecting client!");
        let uaidc = uaid.clone();
        let uidc = uid.clone();
        let mut clients = self.clients.write().await;
        let client_exists = clients
            .get(&uaidc)
            .map_or(false, |client| client.uid == uidc);
        if client_exists {
            clients.remove(&uaidc).expect("Couldn't remove client?");
            return Ok(());
        }
        Err(ApcErrorKind::GeneralError("User not connected".into()).into())
    }
}
