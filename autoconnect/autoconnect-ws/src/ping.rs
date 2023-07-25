use tokio::time::{interval, Interval};

use autoconnect_common::{broadcast::Broadcast, protocol::ServerMessage};
use autoconnect_settings::Settings;
use autoconnect_ws_sm::WebPushClient;

use crate::{
    error::{WSError, WSErrorKind},
    session::Session,
};

#[derive(Debug)]
enum Waiting {
    /// Waiting to send a WebSocket Ping (or WebPush Broadcast) to the Client
    ToPing,
    /// Waiting for the Client to respond to our WebSocket Ping with a Pong
    ForPong,
}

/// Manages WebSocket Pings sent to the Client
///
/// We automatically send WebSocket Pings (or WebPush Broadcasts, if one is
/// pending) every `auto_ping_interval`. If the Client fails to respond to the
/// Ping with a Pong within the `auto_ping_timeout` interval we drop their
/// connection
#[derive(Debug)]
pub struct PingManager {
    /// Waiting to Ping or timeout recieving a Pong
    waiting: Waiting,
    ping_or_timeout: Interval,
}

impl PingManager {
    pub async fn new(settings: &Settings) -> PingManager {
        // Begin by waiting to Ping
        let mut ping_or_timeout = interval(settings.auto_ping_interval);
        ping_or_timeout.tick().await;
        Self {
            waiting: Waiting::ToPing,
            ping_or_timeout,
        }
    }

    /// Complete the next instant PingManager's scheduled to intervene in a
    /// Client's session
    ///
    /// Signals either a:
    ///
    /// - WebSocket Ping (or a WebPush Broadcast, if one is pending) to be sent
    /// to the Client to prevent its connection from idling out/disconnecting
    /// due to inactivity
    ///
    /// - WebSocket Ping was previously sent and the Client failed to respond
    /// with a Pong within the `auto_ping_timeout` interval
    /// (`WSError::PongTimeout` Error returned)
    pub async fn tick(&mut self) -> Result<(), WSError> {
        self.ping_or_timeout.tick().await;
        match self.waiting {
            Waiting::ToPing => Ok(()),
            Waiting::ForPong => Err(WSErrorKind::PongTimeout.into()),
        }
    }

    /// Send the Client a WebSocket Ping or WebPush Broadcast, if one is pending
    pub async fn ws_ping_or_broadcast(
        &mut self,
        client: &mut WebPushClient,
        session: &mut impl Session,
    ) -> Result<(), WSError> {
        if let Some(broadcasts) = client.broadcast_delta().await {
            let smsg = ServerMessage::Broadcast {
                broadcasts: Broadcast::vec_into_hashmap(broadcasts),
            };
            trace!("📢PingManager::ws_ping_or_broadcast {:#?}", smsg);
            session.text(smsg).await?;
            // Broadcasts don't recieve a Pong but sync against the next Ping
            // anyway
            debug_assert!(matches!(self.waiting, Waiting::ToPing));
            self.ping_or_timeout.reset();
        } else {
            trace!("🏓PingManager::ws_ping_or_broadcast ping");
            session.ping(&[]).await?;
            self.set_waiting(Waiting::ForPong, client.app_settings())
                .await;
        }
        Ok(())
    }

    /// Receive a WebSocket Pong from the Client
    ///
    /// Resetting the timer kicked off from the last WebSocket Ping
    pub async fn on_ws_pong(&mut self, settings: &Settings) {
        trace!(
            "🏓PingManager::on_ws_pong waiting: {:?} -> {:?}",
            self.waiting,
            Waiting::ToPing
        );
        if matches!(self.waiting, Waiting::ForPong) {
            self.set_waiting(Waiting::ToPing, settings).await;
        }
    }

    /// Set the `Waiting` status
    async fn set_waiting(&mut self, waiting: Waiting, settings: &Settings) {
        let period = match waiting {
            Waiting::ToPing => settings.auto_ping_interval,
            Waiting::ForPong => settings.auto_ping_timeout,
        };
        self.waiting = waiting;
        self.ping_or_timeout = interval(period);
        self.ping_or_timeout.tick().await;
    }
}
