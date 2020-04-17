use std::{collections::HashMap, sync::RwLock};

use uuid::Uuid;

use crate::client::RegisteredClient;
use autopush_common::errors::Result;
use super::Notification;
use super::protocol::ServerNotification;

#[derive(Default)]
pub struct ClientRegistry {
    clients: RwLock<HashMap<Uuid, RegisteredClient>>,
}

impl ClientRegistry {
    /// Informs this server that a new `client` has connected
    ///
    /// For now just registers internal state by keeping track of the `client`,
    /// namely its channel to send notifications back.
    pub fn connect(&self, client: RegisteredClient) -> Result<()> {
        debug!("Connecting a client!");
        let mut clients = self.clients.write().map_err(|_| "clients lock poisioned")?;
        if let Some(client) = clients.insert(client.uaid, client) {
            // Drop existing connection
            let result = client.tx.unbounded_send(ServerNotification::Disconnect);
            if result.is_ok() {
                debug!("Told client to disconnect as a new one wants to connect");
            }
        }
        Ok(())
    }

    /// A notification has come for the uaid
    pub fn notify(&self, uaid: Uuid, notif: Notification) -> Result<()> {
        let clients = self.clients.read().map_err(|_| "clients lock poisioned")?;
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
        Err("User not connected".into())
    }

    /// A check for notification command has come for the uaid
    pub fn check_storage(&self, uaid: Uuid) -> Result<()> {
        let clients = self.clients.read().map_err(|_| "clients lock poisioned")?;
        if let Some(client) = clients.get(&uaid) {
            let result = client.tx.unbounded_send(ServerNotification::CheckStorage);
            if result.is_ok() {
                debug!("Told client to check storage");
                return Ok(());
            }
        }
        Err("User not connected".into())
    }

    /// The client specified by `uaid` has disconnected.
    pub fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> Result<()> {
        debug!("Disconnecting client!");
        let mut clients = self.clients.write().map_err(|_| "clients lock poisioned")?;
        let client_exists = clients.get(uaid).map_or(false, |client| client.uid == *uid);
        if client_exists {
            clients.remove(uaid).expect("Couldn't remove client?");
        }
        Ok(())
    }
}
