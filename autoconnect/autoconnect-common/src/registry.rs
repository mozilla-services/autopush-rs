use dashmap::DashMap;
use futures::channel::mpsc;
use uuid::Uuid;

use autopush_common::errors::{ApcErrorKind, Result};
use autopush_common::notification::Notification;

use crate::protocol::ServerNotification;

/// Default capacity for bounded notification channels per client.
const DEFAULT_CHANNEL_CAPACITY: usize = 128;

/// A connected Websocket client.
#[derive(Debug)]
struct RegisteredClient {
    /// The user agent's unique ID.
    #[allow(dead_code)]
    pub uaid: Uuid,
    /// The local ID, used to potentially distinquish multiple UAID connections.
    pub uid: Uuid,
    /// The inbound channel for delivery of locally routed Push Notifications
    pub tx: mpsc::Sender<ServerNotification>,
}

/// Contains a mapping of UAID to the associated RegisteredClient.
pub struct ClientRegistry {
    clients: DashMap<Uuid, RegisteredClient>,
    /// Maximum number of buffered notifications per client before backpressure.
    channel_capacity: usize,
}

impl Default for ClientRegistry {
    fn default() -> Self {
        Self {
            clients: DashMap::new(),
            channel_capacity: DEFAULT_CHANNEL_CAPACITY,
        }
    }
}

impl ClientRegistry {
    /// Create a new ClientRegistry with a configurable channel capacity.
    pub fn with_channel_capacity(channel_capacity: usize) -> Self {
        Self {
            clients: DashMap::new(),
            channel_capacity,
        }
    }

    /// Informs this server that a new `client` has connected
    ///
    /// For now just registers internal state by keeping track of the `client`,
    /// namely its channel to send notifications back.
    pub fn connect(&self, uaid: Uuid, uid: Uuid) -> mpsc::Receiver<ServerNotification> {
        trace!("ClientRegistry::connect");
        let (tx, snotif_stream) = mpsc::channel(self.channel_capacity);
        let client = RegisteredClient { uaid, uid, tx };
        if let Some(old_client) = self.clients.insert(uaid, client) {
            // Drop existing connection
            let result = old_client
                .tx
                .clone()
                .try_send(ServerNotification::Disconnect);
            if result.is_ok() {
                debug!("ClientRegistry::connect Ghosting client, new one wants to connect");
            }
        }
        snotif_stream
    }

    /// A notification has come for the uaid
    pub fn notify(&self, uaid: Uuid, notif: Notification) -> Result<()> {
        trace!("ClientRegistry::notify");
        let Some(client) = self.clients.get(&uaid) else {
            return Err(ApcErrorKind::ClientNotConnected.into());
        };
        debug!("ClientRegistry::notify Found a client to deliver a notification to");
        match client
            .tx
            .clone()
            .try_send(ServerNotification::Notification(notif))
        {
            Ok(()) => {
                debug!("ClientRegistry::notify Dropped notification in queue");
                Ok(())
            }
            Err(e) if e.is_full() => Err(ApcErrorKind::ChannelFull.into()),
            Err(_) => Err(ApcErrorKind::ClientNotConnected.into()),
        }
    }

    /// A check for notification command has come for the uaid
    pub fn check_storage(&self, uaid: Uuid) -> Result<()> {
        trace!("ClientRegistry::check_storage");
        let Some(client) = self.clients.get(&uaid) else {
            return Err(ApcErrorKind::ClientNotConnected.into());
        };
        match client.tx.clone().try_send(ServerNotification::CheckStorage) {
            Ok(()) => {
                debug!("ClientRegistry::check_storage Told client to check storage");
                Ok(())
            }
            Err(e) if e.is_full() => Err(ApcErrorKind::ChannelFull.into()),
            Err(_) => Err(ApcErrorKind::ClientNotConnected.into()),
        }
    }

    /// The client specified by `uaid` has disconnected.
    pub fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> Result<()> {
        trace!("ClientRegistry::disconnect");
        let client_exists = self
            .clients
            .get(uaid)
            .is_some_and(|client| client.uid == *uid);
        if client_exists {
            self.clients
                .remove(uaid)
                .ok_or_else(|| ApcErrorKind::GeneralError("Couldn't remove client".into()))?;
            return Ok(());
        }
        Err(ApcErrorKind::ClientNotConnected.into())
    }

    pub fn count(&self) -> usize {
        self.clients.len()
    }
}
