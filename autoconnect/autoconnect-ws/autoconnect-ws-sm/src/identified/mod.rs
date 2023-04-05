use std::{fmt, sync::Arc};

use cadence::Timed;
use uuid::Uuid;

use autoconnect_common::protocol::ServerMessage;
use autoconnect_settings::AppState;
use autopush_common::{
    db::{HelloResponse, User},
    notification::Notification,
    util::{ms_since_epoch, user_agent::UserAgentInfo},
};

use crate::error::SMError;

mod on_client_msg;
mod on_server_notif;

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
            .field(
                "deferred_user_registration",
                &self.deferred_user_registration,
            )
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
        assert!(matches!(pong.as_slice(), [ServerMessage::Ping]));
    }
}
