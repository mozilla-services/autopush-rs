use std::collections::HashMap;

use futures::{
    future::{err, ok},
};
use futures_locks::RwLock;
use uuid::Uuid;

use super::protocol::ServerNotification;
use super::Notification;
// use crate::old_client::RegisteredClient;
use crate::aclient::RegisteredClient;
use autopush_common::errors::{ApiError, ApiResult};

/// Hash of attached clients for this node, mapped by uaid (type:uuid).
#[derive(Default)]
pub struct ClientRegistry {
    clients: RwLock<HashMap<Uuid, RegisteredClient>>,
}

impl ClientRegistry {
    /// Informs this server that a new `client` has connected
    ///
    /// For now just registers internal state by keeping track of the `client`,
    /// namely its channel to send notifications back.
    pub async fn connect(&self, client: RegisteredClient) -> ApiResult<()> {
        debug!("Connecting a client!");
        let mut clients = self.clients
            .write()
            .await;
            if let Some(client) = clients.insert(client.uaid, client) {
                // Drop existing connection
                let result = client.tx.unbounded_send(ServerNotification::Disconnect);
                if result.is_ok() {
                    debug!("Told client to disconnect as a new one wants to connect");
                    return Ok(())

                } else {
                    debug!("Could not send to client");
                    return Err(ApiError::from("clients lock poisoned"))
                }
            }
            Err(ApiError::from("clients lock poisoned"))
    }

    /// A notification has come for the uaid
    pub async fn notify(&self, uaid: Uuid, notif: Notification) -> ApiResult<()> {
        let clients = self
            .clients
            .read()
            .await;

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
            Err(ApiError::from("User not connected"))
    }

    /// A check for notification command has come for the uaid
    pub async fn check_storage(&self, uaid: Uuid) -> ApiResult<()> {
        let clients = self
            .clients
            .read()
            .await;
        if let Some(client) = clients.get(&uaid) {
            let result = client.tx.unbounded_send(ServerNotification::CheckStorage);
            if result.is_ok() {
                debug!("Told client to check storage");
                return Ok(());
            }
        }
        Err(ApiError::from("User not connected"))
    }

    /// The client specified by `uaid` has disconnected.
    #[allow(clippy::clone_on_copy)]
    pub async fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> ApiResult<()> {
        debug!("Disconnecting client!");
        let uaidc = uaid.clone();
        let uidc = uid.clone();
        let mut clients = self
            .clients
            .write()
            .await;
        let client_exists = clients
            .get(&uaidc)
            .map_or(false, |client| client.uid == uidc);
        if client_exists {
            clients.remove(&uaidc).expect("Couldn't remove client?");
            return Ok(());
        }
        Err(ApiError::from("User not connected"))
    }
}
