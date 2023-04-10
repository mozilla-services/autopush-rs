use autoconnect_common::protocol::{ServerMessage, ServerNotification};
use autopush_common::notification::Notification;

use super::WebPushClient;
use crate::error::SMError;

impl WebPushClient {
    pub async fn on_server_notif(
        &mut self,
        snotif: ServerNotification,
    ) -> Result<Vec<ServerMessage>, SMError> {
        match snotif {
            ServerNotification::CheckStorage => self.check_storage().await,
            ServerNotification::Notification(notif) => Ok(vec![self.notif(notif).await?]),
            ServerNotification::Disconnect => Err(SMError::Ghost),
        }
    }

    /// Move queued push notifications to unacked_direct_notifs (to be stored
    /// in the db)
    pub fn on_server_notif_shutdown(&mut self, snotif: ServerNotification) {
        if let ServerNotification::Notification(notif) = snotif {
            self.ack_state.unacked_direct_notifs.push(notif);
        }
    }

    pub(super) async fn check_storage(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        // TODO:
        Ok(vec![])
    }

    async fn notif(&mut self, notif: Notification) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient::notif Sending a direct notif");
        if notif.ttl != 0 {
            self.ack_state.unacked_direct_notifs.push(notif.clone());
        }
        Ok(ServerMessage::Notification(notif))
    }
}
