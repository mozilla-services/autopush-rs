use std::{collections::HashMap, fmt, sync::Arc};

use cadence::{CountedExt, Timed};
use uuid::Uuid;

use autoconnect_common::protocol::{ClientAck, ClientMessage, ServerMessage, ServerNotification};
use autoconnect_settings::AppState;
use autopush_common::{
    db::{HelloResponse, User},
    endpoint::make_endpoint,
    notification::Notification,
    util::{ms_since_epoch, sec_since_epoch, user_agent::UserAgentInfo},
};

use crate::error::SMError;

/// TODO: more docs in this module
pub struct WebPushClient {
    /// Push User Agent identifier. Each Push client recieves a unique UAID
    pub uaid: Uuid,
    /// Unique, local (to each autoconnect instance) identifier
    pub uid: Uuid,
    /// The User Agent information block derived from the User-Agent header
    ua_info: UserAgentInfo,

    //broadcast_subs: BroadcastSubs,
    flags: ClientFlags,
    ack_state: AckState,
    /// Count of messages sent from storage (for enforcing
    /// `settings.msg_limit`). Resets to 0 when storage is emptied.
    #[allow(dead_code)]
    sent_from_storage: u32,
    /// Exists when we didn't register this new user during Hello
    deferred_user_registration: Option<User>,

    stats: SessionStatistics,

    /// Timestamp of when the UA connected (used by database lookup, thus u64)
    connected_at: u64,
    /// Timestamp of the last WebPush Ping message
    last_ping: u64,

    pub app_state: Arc<AppState>,
}

impl fmt::Debug for WebPushClient {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("WebPushClient")
            .field("uaid", &self.uaid)
            .field("uid", &self.uid)
            .field("ua_info", &self.ua_info)
            .field("flags", &self.flags)
            .field("ack_state", &self.ack_state)
            .field("sent_from_storage", &self.sent_from_storage)
            .field("stats", &self.stats)
            .field("connected_at", &self.connected_at)
            .field("last_ping", &self.last_ping)
            .finish()
    }
}

impl WebPushClient {
    pub async fn new(
        uaid: Uuid,
        ua: String,
        flags: ClientFlags,
        connected_at: u64,
        deferred_user_registration: Option<User>,
        app_state: Arc<AppState>,
    ) -> Result<(Self, Vec<ServerMessage>), SMError> {
        let mut client = WebPushClient {
            uaid,
            uid: Uuid::new_v4(),
            ua_info: UserAgentInfo::from(ua.as_str()),
            flags,
            ack_state: Default::default(),
            sent_from_storage: Default::default(),
            connected_at,
            deferred_user_registration,
            last_ping: Default::default(),
            stats: Default::default(),
            app_state,
        };
        let smsgs = if client.flags.check_storage {
            client.check_storage().await?
        } else {
            vec![]
        };
        Ok((client, smsgs))
    }

    pub async fn on_client_msg(
        &mut self,
        msg: ClientMessage,
    ) -> Result<Vec<ServerMessage>, SMError> {
        match msg {
            ClientMessage::Hello { .. } => {
                Err(SMError::InvalidMessage("Already Hello'd".to_owned()))
            }
            ClientMessage::Register { channel_id, key } => {
                Ok(vec![self.register(channel_id, key).await?])
            }
            ClientMessage::Unregister { channel_id, code } => {
                Ok(vec![self.unregister(channel_id, code).await?])
            }
            ClientMessage::BroadcastSubscribe { broadcasts } => Ok(self
                .broadcast_subscribe(broadcasts)
                .await?
                .map_or_else(Vec::new, |smsg| vec![smsg])),
            ClientMessage::Ack { updates } => self.ack(&updates).await,
            ClientMessage::Nack { code, .. } => {
                self.nack(code).await?;
                Ok(vec![])
            }
            ClientMessage::Ping => Ok(vec![self.ping().await?]),
        }
    }

    async fn register(
        &mut self,
        channel_id_str: String,
        key: Option<String>,
    ) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient:register";
               "uaid" => &self.uaid.to_string(),
               "channel_id" => &channel_id_str,
               "key" => &key,
        );
        let channel_id = Uuid::try_parse(&channel_id_str)
            .map_err(|_| SMError::InvalidMessage(format!("Invalid channelID: {channel_id_str}")))?;
        if channel_id.as_hyphenated().to_string() != channel_id_str {
            return Err(SMError::InvalidMessage(format!(
                "Invalid UUID format, not lower-case/dashed: {channel_id}",
            )));
        }

        let smsg = match self._register(&channel_id, key).await {
            Ok(endpoint) => {
                let _ = self.app_state.metrics.incr("ua.command.register");
                self.stats.registers += 1;
                ServerMessage::Register {
                    channel_id,
                    status: 200,
                    push_endpoint: endpoint,
                }
            }
            Err(e) => {
                error!("WebPushClient::register failed: {}", e);
                ServerMessage::Register {
                    channel_id,
                    status: 500,
                    push_endpoint: "".to_owned(),
                }
            }
        };
        Ok(smsg)
    }

    async fn _register(
        &mut self,
        channel_id: &Uuid,
        key: Option<String>,
    ) -> Result<String, SMError> {
        if let Some(user) = &self.deferred_user_registration {
            trace!(
                "💬WebPushClient::register: User not yet registered... {:?}",
                &user.uaid
            );
            self.app_state.db.add_user(user).await?;
            self.deferred_user_registration = None;
        }

        let endpoint = make_endpoint(
            &self.uaid,
            channel_id,
            key.as_deref(),
            &self.app_state.endpoint_url,
            &self.app_state.fernet,
        )
        .map_err(|e| SMError::MakeEndpoint(e.to_string()))?;
        self.app_state
            .db
            .add_channel(&self.uaid, channel_id)
            .await?;
        Ok(endpoint)
    }

    async fn unregister(
        &mut self,
        channel_id: Uuid,
        code: Option<u32>,
    ) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient:unregister";
               "uaid" => &self.uaid.to_string(),
               "channel_id" => &channel_id.to_string(),
               "code" => &code,
        );
        // TODO: (copied from previous state machine) unregister should check
        // the format of channel_id like register does

        let result = self
            .app_state
            .db
            .remove_channel(&self.uaid, &channel_id)
            .await;
        let smsg = match result {
            Ok(_) => {
                self.app_state
                    .metrics
                    .incr_with_tags("ua.command.unregister")
                    .with_tag("code", &code.unwrap_or(200).to_string())
                    .send();
                self.stats.unregisters += 1;
                ServerMessage::Unregister {
                    channel_id,
                    status: 200,
                }
            }
            Err(e) => {
                error!("WebPushClient::unregister failed: {}", e);
                ServerMessage::Unregister {
                    channel_id,
                    status: 500,
                }
            }
        };
        Ok(smsg)
    }

    async fn broadcast_subscribe(
        &mut self,
        _broadcasts: HashMap<String, String>,
    ) -> Result<Option<ServerMessage>, SMError> {
        unimplemented!();
    }

    async fn ack(&mut self, _updates: &[ClientAck]) -> Result<Vec<ServerMessage>, SMError> {
        // TODO:
        self.maybe_post_process_acks().await
    }

    async fn nack(&mut self, _code: Option<i32>) -> Result<(), SMError> {
        unimplemented!();
    }

    async fn ping(&mut self) -> Result<ServerMessage, SMError> {
        // TODO: why is this 45 vs the comment describing a minute?
        // Clients shouldn't ping > than once per minute or we disconnect them
        if sec_since_epoch() - self.last_ping >= 45 {
            trace!("🏓WebPushClient Got a WebPush Ping, sending WebPush Pong");
            Ok(ServerMessage::Ping)
        } else {
            Err(SMError::ExcessivePing)
        }
    }

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

    async fn check_storage(&mut self) -> Result<Vec<ServerMessage>, SMError> {
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

    async fn maybe_post_process_acks(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        if self.ack_state.unacked_notifs() {
            // Waiting for the Client to Ack all notifications it's been sent
            // before further processing
            return Ok(vec![]);
        }

        // TODO:
        let flags = &self.flags;
        if flags.check_storage && flags.increment_storage {
            trace!("WebPushClient:maybe_post_process_acks check_storage && increment_storage");
            unimplemented!()
        } else if flags.check_storage {
            trace!("WebPushClient:maybe_post_process_acks check_storage");
            self.check_storage().await
        } else if flags.rotate_message_table {
            trace!("WebPushClient:maybe_post_process_acks rotate_message_table");
            unimplemented!()
        } else if flags.reset_uaid {
            trace!("WebPushClient:maybe_post_process_acks reset_uaid");
            self.app_state.db.remove_user(&self.uaid).await?;
            Err(SMError::UaidReset)
        } else {
            Ok(vec![])
        }
    }

    /// Move queued push notifications to unacked_direct_notifs (to be stored
    /// in the db)
    pub fn on_server_notif_shutdown(&mut self, snotif: ServerNotification) {
        if let ServerNotification::Notification(notif) = snotif {
            self.ack_state.unacked_direct_notifs.push(notif);
        }
    }

    pub fn shutdown(&mut self) {
        // TODO: logging
        let now = ms_since_epoch();
        let elapsed = (now - self.connected_at) / 1_000;
        self.app_state
            .metrics
            .time_with_tags("ua.connection.lifespan", elapsed)
            .with_tag("ua_os_family", &self.ua_info.metrics_os)
            .with_tag("ua_browser_family", &self.ua_info.metrics_browser)
            .send();

        // TODO: save unacked notifs, logging
    }
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ClientFlags {
    /// Whether check_storage queries for topic (not "timestamped") messages
    include_topic: bool,
    /// Flags the need to increment the last read for timestamp for timestamped messages
    increment_storage: bool,
    /// Whether this client needs to check storage for messages
    check_storage: bool,
    /// Flags the need to drop the user record
    reset_uaid: bool,
    rotate_message_table: bool,
}

impl ClientFlags {
    pub fn from_hello(hello_response: &HelloResponse) -> Self {
        Self {
            check_storage: hello_response.check_storage,
            reset_uaid: hello_response.reset_uaid,
            rotate_message_table: hello_response.rotate_message_table,
            ..Default::default()
        }
    }
}

impl Default for ClientFlags {
    fn default() -> Self {
        Self {
            include_topic: true,
            increment_storage: false,
            check_storage: false,
            reset_uaid: false,
            rotate_message_table: false,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct SessionStatistics {
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

/// Record of Notifications sent to the Client.
#[allow(dead_code)]
#[derive(Debug, Default)]
struct AckState {
    /// List of unAck'd directly sent (never stored) notifications
    unacked_direct_notifs: Vec<Notification>,
    /// List of unAck'd sent notifications from storage
    unacked_stored_notifs: Vec<Notification>,
    /// Id of the last previously unAck'd, stored transmission
    /// TODO: better docs
    unacked_stored_highest: Option<u64>,
}

impl AckState {
    fn unacked_notifs(&self) -> bool {
        !self.unacked_stored_notifs.is_empty() || !self.unacked_direct_notifs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use autoconnect_common::{
        protocol::{ClientMessage, ServerMessage},
        test_support::{DUMMY_UAID, UA},
    };
    use autoconnect_settings::AppState;
    use autopush_common::util::ms_since_epoch;

    use super::WebPushClient;

    async fn wpclient(uaid: Uuid, app_state: AppState) -> (WebPushClient, Vec<ServerMessage>) {
        WebPushClient::new(
            uaid,
            UA.to_owned(),
            Default::default(),
            ms_since_epoch(),
            None,
            Arc::new(app_state),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn webpush_ping() {
        let (mut client, _) = wpclient(DUMMY_UAID, Default::default()).await;
        let pong = client.on_client_msg(ClientMessage::Ping).await.unwrap();
        assert!(matches!(pong[0], ServerMessage::Ping));
        assert_eq!(pong.len(), 1);
    }
}
