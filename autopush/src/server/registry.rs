use std::collections::HashMap;

use futures::{
    future::{err, ok},
    Future,
};
use futures_locks::RwLock;
use uuid::Uuid;

use super::protocol::ServerNotification;
use super::Notification;
use crate::client::RegisteredClient;
use autopush_common::errors::{ApcError, ApcErrorKind};

/// `notify` + `check_storage` are used under hyper (http.rs) which `Send`
/// futures.
///
/// `MySendFuture` is `Send` for this, similar to modern futures `BoxFuture`.
/// `MyFuture` isn't, similar to modern futures `BoxLocalFuture`
pub type MySendFuture<T> = Box<dyn Future<Item = T, Error = ApcError> + Send>;

#[derive(Default)]
pub struct ClientRegistry {
    clients: RwLock<HashMap<Uuid, RegisteredClient>>,
}

impl ClientRegistry {
    /// Informs this server that a new `client` has connected
    ///
    /// For now just registers internal state by keeping track of the `client`,
    /// namely its channel to send notifications back.
    pub fn connect(&self, client: RegisteredClient) -> MySendFuture<()> {
        debug!("Connecting a client!");
        Box::new(
            self.clients
                .write()
                .and_then(|mut clients| {
                    if let Some(client) = clients.insert(client.uaid, client) {
                        // Drop existing connection
                        let result = client.tx.unbounded_send(ServerNotification::Disconnect);
                        if result.is_ok() {
                            debug!("Told client to disconnect as a new one wants to connect");
                        }
                    }
                    ok(())
                })
                .map_err(|_| ApcErrorKind::GeneralError("clients lock poisoned".into()).into()),
        )

        //.map_err(|_| "clients lock poisioned")
    }

    /// A notification has come for the uaid
    pub fn notify(&self, uaid: Uuid, notif: Notification) -> MySendFuture<()> {
        let fut = self
            .clients
            .read()
            .and_then(move |clients| {
                debug!("Sending notification");
                if let Some(client) = clients.get(&uaid) {
                    debug!("Found a client to deliver a notification to");
                    let result = client
                        .tx
                        .unbounded_send(ServerNotification::Notification(notif));
                    if result.is_ok() {
                        debug!("Dropped notification in queue");
                        return ok(());
                    }
                }
                err(())
            })
            .map_err(|_| ApcErrorKind::GeneralError("User not connected".into()).into());
        Box::new(fut)
    }

    /// A check for notification command has come for the uaid
    pub fn check_storage(&self, uaid: Uuid) -> MySendFuture<()> {
        let fut = self
            .clients
            .read()
            .and_then(move |clients| {
                if let Some(client) = clients.get(&uaid) {
                    let result = client.tx.unbounded_send(ServerNotification::CheckStorage);
                    if result.is_ok() {
                        debug!("Told client to check storage");
                        return ok(());
                    }
                }
                err(())
            })
            .map_err(|_| ApcErrorKind::GeneralError("User not connected".into()).into());
        Box::new(fut)
    }

    /// The client specified by `uaid` has disconnected.
    #[allow(clippy::clone_on_copy)]
    pub fn disconnect(&self, uaid: &Uuid, uid: &Uuid) -> MySendFuture<()> {
        debug!("Disconnecting client!");
        let uaidc = uaid.clone();
        let uidc = uid.clone();
        let fut = self
            .clients
            .write()
            .and_then(move |mut clients| {
                let client_exists = clients
                    .get(&uaidc)
                    .map_or(false, |client| client.uid == uidc);
                if client_exists {
                    clients.remove(&uaidc).expect("Couldn't remove client?");
                    return ok(());
                }
                err(())
            })
            .map_err(|_| ApcErrorKind::GeneralError("User not connected".into()).into());
        Box::new(fut)
    }
}
