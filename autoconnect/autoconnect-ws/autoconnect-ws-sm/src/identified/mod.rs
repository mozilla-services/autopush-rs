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
        trace!("WebPushClient::new: Initial smsgs count: {}", smsgs.len());
        Ok((client, smsgs))
    }

    pub fn shutdown(mut self) {
        // TODO: logging
        let now = ms_since_epoch();
        let elapsed = (now - self.connected_at) / 1_000;
        self.app_state
            .metrics
            .time_with_tags("ua.connection.lifespan", elapsed)
            .with_tag("ua_os_family", &self.ua_info.metrics_os)
            .with_tag("ua_browser_family", &self.ua_info.metrics_browser)
            .send();

        // Save any unAck'd Direct notifs
        if !self.ack_state.unacked_direct_notifs.is_empty() {
            self.save_and_notify_undelivered_notifs();
        }
    }

    fn save_and_notify_undelivered_notifs(&mut self) {
        use std::mem;
        use std::time::Duration;
        use actix_web::rt;

        let mut notifs = mem::take(&mut self.ack_state.unacked_direct_notifs);
        self.stats.direct_storage += notifs.len() as i32;
        // XXX: Ensure we don't store these as legacy by setting a 0 as the
        // sortkey_timestamp. This ensures the Python side doesn't mark it as
        // legacy during conversion and still get the correct default us_time
        // when saving
        for notif in &mut notifs {
            notif.sortkey_timestamp = Some(0);
        }

        let db = self.app_state.db.clone();
        let uaid = self.uaid;
        let connected_at = self.connected_at;
        rt::spawn(async move {
            db.save_messages(&uaid, notifs).await?;
            debug!("Finished saving unacked direct notifications, checking for reconnect");
            let Some(user) = db.get_user(&uaid).await? else {
                // XXX:
                panic!();
            };
            if connected_at == user.connected_at {
                return Ok(());
            }
            if let Some(node_id) = user.node_id {
                // XXX: app_state.reqwest
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(1))
                    .build()
                    .map_err(|e| SMError::Internal("XXX".to_owned()))?;
                let notify_url = format!("{}/notif/{}", node_id, uaid.as_simple());
                client.put(&notify_url).send().await?.error_for_status()?;
            }
            Ok::<(), SMError>(())
        });
    }
}

use autopush_common::db::{client::DbClient, error::DbResult, USER_RECORD_VERSION};
// XXX: likely simpler for this to reside in the Db trait
pub async fn process_existing_user(
    db: &Box<dyn DbClient>,
    mut user: User,
) -> DbResult<(User, ClientFlags)> {
    // XXX: could verify these aren't empty on ddb
    let message_tables = db.message_tables();
    if !message_tables.is_empty()
        && (user.current_month.is_none()
            || !message_tables.contains(&user.current_month.as_ref().unwrap()))
    {
        db.remove_user(&user.uaid).await?;
        // XXX: no current_month/node_id are set in this case! return None instead?
        user = Default::default();
    }
    let flags = ClientFlags {
        check_storage: true,
        reset_uaid: user
            .record_version
            .map_or(true, |rec_ver| rec_ver < USER_RECORD_VERSION),
        rotate_message_table: user.current_month != db.current_message_month(),
        ..Default::default()
    };
    user.set_last_connect();
    Ok((user, flags))
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
    /// Number of acknowledged messages that were sent directly (not via storage)
    direct_acked: i32,
    /// Number of messages sent to storage
    direct_storage: i32,
    /// Number of messages taken from storage
    stored_retrieved: i32,
    /// Number of message pulled from storage and acknowledged
    stored_acked: i32,
    /// Number of messages total that are not acknowledged.
    nacks: i32,
    /// Number of unregister requests
    unregisters: i32,
    /// Number of register requests
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

    use autopush_common::notification::Notification;
    fn make_webpush_notif(channel_id: &Uuid, ttl: u64) -> Notification {
        use autopush_common::util::sec_since_epoch;
        Notification {
            channel_id: channel_id.clone(),
            ttl,
            timestamp: sec_since_epoch(),
            ..Default::default()
        }
    }

    #[actix_rt::test]
    async fn webpush_ping() {
        let (mut client, _) = wpclient(DUMMY_UAID, Default::default()).await;
        let pong = client.on_client_msg(ClientMessage::Ping).await.unwrap();
        assert!(matches!(pong.as_slice(), [ServerMessage::Ping]));
    }

    #[actix_rt::test]
    async fn check_storage_stuff() {
        use autopush_common::logging::init_logging;
        init_logging(false).unwrap();
        //let _ = slog_envlogger::init();
        use autoconnect_common::protocol::ServerNotification;
        use autopush_common::db::{client::FetchMessageResponse, mock::MockDbClient};
        let mut db = MockDbClient::new();
        let channel_id = Uuid::new_v4();
        db.expect_fetch_messages()
            .return_once(move |_uaid, _limit| {
                Ok(FetchMessageResponse {
                    timestamp: None,
                    messages: vec![Default::default(), make_webpush_notif(&channel_id, 0)],
                })
            });

        let (mut client, _) = wpclient(
            DUMMY_UAID,
            AppState {
                db: db.into_boxed_arc(),
                ..Default::default()
            },
        )
        .await;

        let smsgs = client
            .on_server_notif(ServerNotification::CheckStorage)
            .await
            .unwrap();
        assert!(matches!(smsgs.as_slice(), []));
    }
}
