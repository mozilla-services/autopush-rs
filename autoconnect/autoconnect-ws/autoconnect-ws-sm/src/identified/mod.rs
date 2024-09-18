use std::{fmt, mem, sync::Arc};

use actix_web::rt;
use cadence::Timed;
use futures::channel::mpsc;
use uuid::Uuid;

use autoconnect_common::{
    broadcast::{Broadcast, BroadcastSubs},
    protocol::{ServerMessage, ServerNotification},
};

use autoconnect_settings::{AppState, Settings};
use autopush_common::{
    db::User,
    notification::Notification,
    util::{ms_since_epoch, user_agent::UserAgentInfo},
};

use crate::error::{SMError, SMErrorKind};

mod on_client_msg;
mod on_server_notif;

/// A WebPush Client that's successfully identified itself to the server via a
/// Hello message.
///
/// The `webpush_ws` handler feeds input from both the WebSocket connection
/// (`ClientMessage`) and the `ClientRegistry` (`ServerNotification`)
/// triggered by autoendpoint to this type's `on_client_msg` and
/// `on_server_notif` methods whose impls reside in their own modules.
///
/// Note the `check_storage` method (in the `on_server_notif` module) is
/// triggered by both a `ServerNotification` and also the `new` constructor
pub struct WebPushClient {
    /// Push User Agent identifier. Each Push client recieves a unique UAID
    pub uaid: Uuid,
    /// Unique, local (to each autoconnect instance) identifier
    pub uid: Uuid,
    /// The User Agent information block derived from the User-Agent header
    pub ua_info: UserAgentInfo,

    /// Broadcast Subscriptions this Client is subscribed to
    broadcast_subs: BroadcastSubs,

    /// Set of session specific flags
    flags: ClientFlags,
    /// Notification Ack(knowledgement) related state
    ack_state: AckState,
    /// Count of messages sent from storage (for enforcing
    /// `settings.msg_limit`). Resets to 0 when storage is emptied
    sent_from_storage: u32,
    /// Exists for new User records: these are not written to the db during
    /// Hello, instead they're lazily added to the db on their first Register
    /// message
    deferred_add_user: Option<User>,

    /// WebPush Session Statistics
    stats: SessionStatistics,

    /// Timestamp of when the UA connected (used by database lookup, thus u64)
    connected_at: u64,
    /// Timestamp of the last WebPush Ping message
    last_ping: u64,
    /// The last notification timestamp.
    // TODO: RENAME THIS TO `last_notification_timestamp`
    current_timestamp: Option<u64>,

    app_state: Arc<AppState>,
}

impl fmt::Debug for WebPushClient {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("WebPushClient")
            .field("uaid", &self.uaid)
            .field("uid", &self.uid)
            .field("ua_info", &self.ua_info)
            .field("broadcast_subs", &self.broadcast_subs)
            .field("flags", &self.flags)
            .field("ack_state", &self.ack_state)
            .field("sent_from_storage", &self.sent_from_storage)
            .field("deferred_add_user", &self.deferred_add_user)
            .field("stats", &self.stats)
            .field("connected_at", &self.connected_at)
            .field("last_ping", &self.last_ping)
            .finish()
    }
}

impl WebPushClient {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        uaid: Uuid,
        ua: String,
        broadcast_subs: BroadcastSubs,
        flags: ClientFlags,
        connected_at: u64,
        current_timestamp: Option<u64>,
        deferred_add_user: Option<User>,
        app_state: Arc<AppState>,
    ) -> Result<(Self, Vec<ServerMessage>), SMError> {
        trace!("👁‍🗨WebPushClient::new");
        let stats = SessionStatistics {
            existing_uaid: deferred_add_user.is_none(),
            ..Default::default()
        };
        let mut client = WebPushClient {
            uaid,
            uid: Uuid::new_v4(),
            ua_info: UserAgentInfo::from(ua.as_str()),
            broadcast_subs,
            flags,
            ack_state: Default::default(),
            sent_from_storage: Default::default(),
            connected_at,
            current_timestamp,
            deferred_add_user,
            last_ping: Default::default(),
            stats,
            app_state,
        };

        let smsgs = if client.flags.check_storage {
            let smsgs = client.check_storage().await?;
            debug!(
                "WebPushClient::new: check_storage smsgs.len(): {}",
                smsgs.len()
            );
            smsgs
        } else {
            vec![]
        };
        Ok((client, smsgs))
    }

    /// Return a reference to `AppState`'s `Settings`
    pub fn app_settings(&self) -> &Settings {
        &self.app_state.settings
    }

    /// Connect this `WebPushClient` to the `ClientRegistry`
    ///
    /// Returning a `Stream` of `ServerNotification`s from the `ClientRegistry`
    pub async fn registry_connect(&self) -> mpsc::UnboundedReceiver<ServerNotification> {
        self.app_state.clients.connect(self.uaid, self.uid).await
    }

    /// Disconnect this `WebPushClient` from the `ClientRegistry`
    pub async fn registry_disconnect(&self) {
        // Ignore disconnect (Client wasn't connected) Errors
        let _ = self
            .app_state
            .clients
            .disconnect(&self.uaid, &self.uid)
            .await;
    }

    /// Return the difference between the Client's Broadcast Subscriptions and
    /// the this server's Broadcasts
    pub async fn broadcast_delta(&mut self) -> Option<Vec<Broadcast>> {
        self.app_state
            .broadcaster
            .read()
            .await
            .change_count_delta(&mut self.broadcast_subs)
    }

    /// Cleanup after the session has ended
    pub fn shutdown(&mut self, reason: Option<String>) {
        trace!("👁‍🗨WebPushClient::shutdown");
        self.save_and_notify_unacked_direct_notifs();

        let ua_info = &self.ua_info;
        let stats = &self.stats;
        let elapsed_sec = (ms_since_epoch() - self.connected_at) / 1_000;
        self.app_state
            .metrics
            .time_with_tags("ua.connection.lifespan", elapsed_sec)
            .with_tag("ua_os_family", &ua_info.metrics_os)
            .with_tag("ua_browser_family", &ua_info.metrics_browser)
            .send();

        // Log out the final stats message
        info!("Session";
            "uaid_hash" => self.uaid.as_simple().to_string(),
            "uaid_reset" => self.flags.old_record_version,
            "existing_uaid" => stats.existing_uaid,
            "connection_type" => "webpush",
            "ua_name" => &ua_info.browser_name,
            "ua_os_family" => &ua_info.metrics_os,
            "ua_os_ver" => &ua_info.os_version,
            "ua_browser_family" => &ua_info.metrics_browser,
            "ua_browser_ver" => &ua_info.browser_version,
            "ua_category" => &ua_info.category,
            "connection_time" => elapsed_sec,
            "direct_acked" => stats.direct_acked,
            "direct_storage" => stats.direct_storage,
            "stored_retrieved" => stats.stored_retrieved,
            "stored_acked" => stats.stored_acked,
            "nacks" => stats.nacks,
            "registers" => stats.registers,
            "unregisters" => stats.unregisters,
            "disconnect_reason" => reason.unwrap_or_else(|| "".to_owned()),
        );
    }

    /// Save any Direct unAck'd messages to the db (on shutdown)
    ///
    /// Direct messages are solely stored in memory until Ack'd by the Client,
    /// so on shutdown, any not Ack'd are stored in the db to not be lost
    fn save_and_notify_unacked_direct_notifs(&mut self) {
        let mut notifs = mem::take(&mut self.ack_state.unacked_direct_notifs);
        trace!(
            "👁‍🗨WebPushClient::save_and_notify_unacked_direct_notifs len: {}",
            notifs.len()
        );
        if notifs.is_empty() {
            return;
        }

        self.stats.direct_storage += notifs.len() as i32;
        // TODO: clarify this comment re the Python version
        // Ensure we don't store these as legacy by setting a 0 as the
        // sortkey_timestamp. This ensures the Python side doesn't mark it as
        // legacy during conversion and still get the correct default us_time
        // when saving
        for notif in &mut notifs {
            notif.sortkey_timestamp = Some(0);
        }

        let app_state = Arc::clone(&self.app_state);
        let uaid = self.uaid;
        let connected_at = self.connected_at;
        rt::spawn(async move {
            app_state.db.save_messages(&uaid, notifs).await?;
            debug!("Finished saving unacked direct notifs, checking for reconnect");
            let Some(user) = app_state.db.get_user(&uaid).await? else {
                return Err(SMErrorKind::Internal(format!(
                    "User not found for unacked direct notifs: {uaid}"
                )));
            };
            if connected_at == user.connected_at {
                return Ok(());
            }
            if let Some(node_id) = user.node_id {
                app_state
                    .http
                    .put(format!("{}/notif/{}", node_id, uaid.as_simple()))
                    .send()
                    .await?
                    .error_for_status()?;
            }
            Ok(())
        });
    }

    /// Add User information and tags for this Client to a Sentry Event
    pub fn add_sentry_info(self, event: &mut sentry::protocol::Event) {
        event.user = Some(sentry::User {
            id: Some(self.uaid.as_simple().to_string()),
            ..Default::default()
        });
        let ua_info = self.ua_info;
        event
            .tags
            .insert("ua_name".to_owned(), ua_info.browser_name);
        event
            .tags
            .insert("ua_os_family".to_owned(), ua_info.metrics_os);
        event
            .tags
            .insert("ua_os_ver".to_owned(), ua_info.os_version);
        event
            .tags
            .insert("ua_browser_family".to_owned(), ua_info.metrics_browser);
        event
            .tags
            .insert("ua_browser_ver".to_owned(), ua_info.browser_version);
    }
}

#[derive(Debug)]
pub struct ClientFlags {
    /// Whether check_storage queries for topic (not "timestamped") messages
    pub include_topic: bool,
    /// Flags the need to increment the last read for timestamp for timestamped messages
    pub increment_storage: bool,
    /// Whether this client needs to check storage for messages
    pub check_storage: bool,
    /// Flags the need to drop the user record
    pub old_record_version: bool,
    /// First time a user has connected "today"
    pub emit_channel_metrics: bool,
}

impl Default for ClientFlags {
    fn default() -> Self {
        Self {
            include_topic: true,
            increment_storage: false,
            check_storage: false,
            old_record_version: false,
            emit_channel_metrics: false,
        }
    }
}

/// WebPush Session Statistics
///
/// Tracks statistics about the session that are logged when the session's
/// closed
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
    /// Whether this uaid was previously registered
    existing_uaid: bool,
}

/// Record of Notifications sent to the Client.
#[derive(Debug, Default)]
struct AckState {
    /// List of unAck'd directly sent (never stored) notifications
    unacked_direct_notifs: Vec<Notification>,
    /// List of unAck'd sent notifications from storage
    unacked_stored_notifs: Vec<Notification>,
    /// Either the `current_timestamp` value in storage (returned from
    /// `fetch_messages`) or the last unAck'd timestamp Message's
    /// `sortkey_timestamp` (returned from `fetch_timestamp_messages`).
    ///
    /// This represents the "pointer" to the beginning (more specifically the
    /// record preceeding the beginning used in a Greater Than query) of the
    /// next batch of timestamp Messages.
    ///
    /// Thus this value is:
    ///
    /// a) initially None, then
    ///
    /// b) retrieved from `current_timestamp` in storage then passed as the
    /// `timestamp` to `fetch_timestamp_messages`. When all of those timestamp
    /// Messages are Ack'd, this value's then
    ///
    /// c) written back to `current_timestamp` in storage via
    /// `increment_storage`
    unacked_stored_highest: Option<u64>,
}

impl AckState {
    /// Whether the Client has outstanding notifications sent to it that it has
    /// yet to Ack
    fn unacked_notifs(&self) -> bool {
        !self.unacked_stored_notifs.is_empty() || !self.unacked_direct_notifs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use uuid::Uuid;

    use autoconnect_common::{
        protocol::{ClientMessage, ServerMessage, ServerNotification},
        test_support::{DUMMY_CHID, DUMMY_UAID, UA},
    };
    use autoconnect_settings::AppState;
    use autopush_common::{
        db::{client::FetchMessageResponse, mock::MockDbClient},
        notification::Notification,
        util::{ms_since_epoch, sec_since_epoch},
    };

    use super::WebPushClient;

    async fn wpclient(uaid: Uuid, app_state: AppState) -> (WebPushClient, Vec<ServerMessage>) {
        WebPushClient::new(
            uaid,
            UA.to_owned(),
            Default::default(),
            Default::default(),
            ms_since_epoch(),
            None,
            None,
            Arc::new(app_state),
        )
        .await
        .unwrap()
    }

    /// Generate a dummy timestamp `Notification`
    fn new_timestamp_notif(channel_id: &Uuid, ttl: u64) -> Notification {
        Notification {
            channel_id: *channel_id,
            ttl,
            timestamp: sec_since_epoch(),
            sortkey_timestamp: Some(ms_since_epoch()),
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
    async fn expired_increments_storage() {
        let mut db = MockDbClient::new();
        let mut seq = mockall::Sequence::new();
        let timestamp = sec_since_epoch();
        // No topic messages
        db.expect_fetch_topic_messages()
            .times(1)
            .in_sequence(&mut seq)
            .return_once(move |_, _| {
                Ok(FetchMessageResponse {
                    timestamp: None,
                    messages: vec![],
                })
            });
        // Return expired notifs (default ttl of 0)
        db.expect_fetch_timestamp_messages()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts, _| ts.is_none())
            .return_once(move |_, _, _| {
                Ok(FetchMessageResponse {
                    timestamp: Some(timestamp),
                    messages: vec![
                        new_timestamp_notif(&DUMMY_CHID, 0),
                        new_timestamp_notif(&DUMMY_CHID, 0),
                    ],
                })
            });
        // EOF
        db.expect_fetch_timestamp_messages()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts, _| ts == &Some(timestamp))
            .return_once(|_, _, _| {
                Ok(FetchMessageResponse {
                    timestamp: None,
                    messages: vec![],
                })
            });
        // Ensure increment_storage's called to advance the timestamp messages
        // despite check_storage returning nothing (all filtered out as
        // expired)
        db.expect_increment_storage()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts| ts == &timestamp)
            .return_once(|_, _| Ok(()));

        // No check_storage called here (via default ClientFlags)
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
            .expect("CheckStorage failed");
        assert!(smsgs.is_empty())
    }
}
