use std::{collections::HashMap, fmt, mem, sync::Arc};

use actix_web::rt;
use cadence::Timed;
use futures::channel::mpsc;
use once_cell::sync::Lazy;
use tokio::sync::Semaphore;
use uuid::Uuid;

/// Limit concurrent disconnect-cleanup tasks to prevent resource exhaustion
/// during disconnect storms (e.g., pod scaling events).
static DISCONNECT_SEMAPHORE: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(64));

use autoconnect_common::{
    broadcast::{Broadcast, BroadcastSubs},
    protocol::{ServerMessage, ServerNotification},
};

use autoconnect_settings::{AppState, Settings};
use autopush_common::{
    db::User,
    metric_name::MetricName,
    metrics::StatsdClientExt,
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

    #[cfg(feature = "reliable_report")]
    pub fn app_reliability(&self) -> &autopush_common::reliability::PushReliability {
        &self.app_state.reliability
    }

    /// Connect this `WebPushClient` to the `ClientRegistry`
    ///
    /// Returning a `Stream` of `ServerNotification`s from the `ClientRegistry`
    pub fn registry_connect(&self) -> mpsc::Receiver<ServerNotification> {
        self.app_state.clients.connect(self.uaid, self.uid)
    }

    /// Disconnect this `WebPushClient` from the `ClientRegistry`
    pub fn registry_disconnect(&self) {
        // Ignore disconnect (Client wasn't connected) Errors
        let _ = self.app_state.clients.disconnect(&self.uaid, &self.uid);
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
        // TTL=0 notifications are delivered immediately or not at all
        self.ack_state.unacked_ttl0_notifs.clear();
        let notif_map = mem::take(&mut self.ack_state.unacked_direct_notifs);
        trace!(
            "👁‍🗨WebPushClient::save_and_notify_unacked_direct_notifs len: {}",
            notif_map.len()
        );
        if notif_map.is_empty() {
            return;
        }

        self.stats.direct_storage += notif_map.len() as i32;
        // TODO: clarify this comment re the Python version
        // Ensure we don't store these as legacy by setting a 0 as the
        // sortkey_timestamp. This ensures the Python side doesn't mark it as
        // legacy during conversion and still get the correct default us_time
        // when saving
        let mut notifs: Vec<Notification> = notif_map.into_values().collect();
        for notif in &mut notifs {
            notif.sortkey_timestamp = Some(0);
        }

        let app_state = Arc::clone(&self.app_state);
        let uaid = self.uaid;
        let connected_at = self.connected_at;
        rt::spawn(async move {
            let _permit = match DISCONNECT_SEMAPHORE.acquire().await {
                Ok(permit) => permit,
                Err(_) => {
                    app_state
                        .metrics
                        .incr(MetricName::ErrorDisconnectSemaphoreFull)
                        .ok();
                    warn!("Disconnect semaphore full, skipping save of unacked direct notifs");
                    return Ok(());
                }
            };
            #[cfg(not(feature = "reliable_report"))]
            app_state.db.save_messages(&uaid, notifs).await?;
            #[cfg(feature = "reliable_report")]
            {
                app_state.db.save_messages(&uaid, notifs.clone()).await?;
                for mut notif in notifs {
                    notif
                        .record_reliability(
                            &app_state.reliability,
                            autopush_common::reliability::ReliabilityState::Stored,
                        )
                        .await;
                }
            }
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

/// Key for looking up notifications in the ACK tracking maps.
/// The `version` field is the `Notification.message_id`, a fernet-encrypted
/// composite of uaid, channel_id, and topic|timestamp. It is globally unique
/// (fernet adds random padding), so `version` alone suffices as the key.
type AckKey = String;

/// A Notification the Client has Ack'd, and when.
#[derive(Debug)]
struct AckedNotification {
    notif: Notification,
    ack_ms: u64,
}

/// Record of Notifications sent to the Client.
#[derive(Debug, Default)]
struct AckState {
    /// Map of unAck'd directly sent (never stored) notifications
    unacked_direct_notifs: HashMap<AckKey, Notification>,
    /// Map of unAck'd directly sent TTL=0 notifications, kept only to record
    /// their delivery on ACK. Unlike `unacked_direct_notifs`, these are never
    /// stored for redelivery.
    unacked_ttl0_notifs: HashMap<AckKey, Notification>,
    /// Map of unAck'd sent notifications from storage
    unacked_stored_notifs: HashMap<AckKey, Notification>,
    /// Ack'd timestamp notifications from storage, with when they were Ack'd.
    /// The whole batch must be Ack'd before messages are removed from storage
    /// via `increment_storage`; if the session ends first, they will be sent
    /// again on reconnect, so their delivery is recorded in `increment_storage`.
    acked_stored_timestamp_notifs: Vec<AckedNotification>,
    /// Expired timestamp notifications skipped while reading storage. Like
    /// `acked_stored_timestamp_notifs`, they're counted in `increment_storage`.
    expired_stored_timestamp_count: i64,
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
    ///
    /// TTL=0 notifications are not included because they are not stored
    /// for redelivery.
    fn unacked_notifs(&self) -> bool {
        !self.unacked_stored_notifs.is_empty() || !self.unacked_direct_notifs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cadence::{SpyMetricSink, StatsdClient};
    use uuid::Uuid;

    use autoconnect_common::{
        protocol::{ClientAck, ClientMessage, ServerMessage, ServerNotification},
        test_support::{DUMMY_CHID, DUMMY_UAID, UA},
    };
    use autoconnect_settings::AppState;
    use autopush_common::{
        db::{client::FetchMessageResponse, error::DbError, mock::MockDbClient},
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

    /// An `AppState` using `db` whose statsd metrics are captured by the
    /// returned receiver
    fn spy_app_state(db: MockDbClient) -> (AppState, impl Fn() -> Vec<String>) {
        let (rx, sink) = SpyMetricSink::new();
        let app_state = AppState {
            db: db.into_boxed_arc(),
            metrics: Arc::new(StatsdClient::from_sink("autoconnect", sink)),
            ..Default::default()
        };
        let packets = move || {
            rx.try_iter()
                .map(|packet| String::from_utf8(packet).unwrap())
                .collect()
        };
        (app_state, packets)
    }

    /// Sum of `notification.outcome` for `outcome` in the statsd `packets`
    fn outcome_count(packets: &[String], outcome: &str) -> i64 {
        let tag = format!("outcome:{outcome},");
        packets
            .iter()
            .filter(|p| p.starts_with("autoconnect.notification.outcome:") && p.contains(&tag))
            .map(|p| {
                p["autoconnect.notification.outcome:".len()..]
                    .split('|')
                    .next()
                    .unwrap()
                    .parse::<i64>()
                    .unwrap()
            })
            .sum()
    }

    /// A topic `Notification` (no `sortkey_timestamp`)
    fn new_topic_notif(channel_id: &Uuid, ttl: u64) -> Notification {
        Notification {
            topic: Some("topic".to_owned()),
            sortkey_timestamp: None,
            ..new_timestamp_notif(channel_id, ttl)
        }
    }

    fn ack(version: &str) -> ClientMessage {
        ClientMessage::Ack {
            updates: vec![ClientAck {
                channel_id: DUMMY_CHID,
                version: version.to_owned(),
                code: Some(100),
            }],
        }
    }

    /// Mock reading one batch of timestamp messages (`v1`, `v2` and an
    /// expired one) at `timestamp`, with no topic messages
    fn expect_timestamp_batch(db: &mut MockDbClient, seq: &mut mockall::Sequence, timestamp: u64) {
        db.expect_fetch_topic_messages()
            .times(1)
            .in_sequence(seq)
            .return_once(|_, _| Ok(FetchMessageResponse::default()));
        db.expect_fetch_timestamp_messages()
            .times(1)
            .in_sequence(seq)
            .withf(|_, ts, _| ts.is_none())
            .return_once(move |_, _, _| {
                Ok(FetchMessageResponse {
                    timestamp: Some(timestamp),
                    messages: vec![
                        Notification {
                            version: "v1".to_owned(),
                            ..new_timestamp_notif(&DUMMY_CHID, 60)
                        },
                        Notification {
                            version: "v2".to_owned(),
                            ..new_timestamp_notif(&DUMMY_CHID, 60)
                        },
                        Notification {
                            version: "expired".to_owned(),
                            ..new_timestamp_notif(&DUMMY_CHID, 0)
                        },
                    ],
                })
            });
    }

    /// A stored batch that's only partly Ack'd before the session ends is
    /// read again on reconnect, so nothing in it is counted yet
    #[actix_rt::test]
    async fn partial_stored_ack_counts_nothing() {
        let mut db = MockDbClient::new();
        let mut seq = mockall::Sequence::new();
        expect_timestamp_batch(&mut db, &mut seq, sec_since_epoch());
        db.expect_increment_storage().never();
        let (app_state, packets) = spy_app_state(db);
        let delivery_metrics = app_state.delivery_metrics.clone();
        let (mut client, _) = wpclient(DUMMY_UAID, app_state).await;

        let smsgs = client
            .on_server_notif(ServerNotification::CheckStorage)
            .await
            .unwrap();
        assert_eq!(smsgs.len(), 2);
        client.on_client_msg(ack("v1")).await.unwrap();
        client.shutdown(None);

        let packets = packets();
        assert_eq!(outcome_count(&packets, "delivered"), 0);
        assert_eq!(outcome_count(&packets, "expired"), 0);
        assert!(
            !delivery_metrics
                .encode()
                .unwrap()
                .contains(r#"route="stored""#)
        );
    }

    /// Once a stored batch is fully Ack'd and storage advances, each message
    /// is counted exactly once
    #[actix_rt::test]
    async fn full_stored_ack_counts_once() {
        let mut db = MockDbClient::new();
        let mut seq = mockall::Sequence::new();
        let timestamp = sec_since_epoch();
        expect_timestamp_batch(&mut db, &mut seq, timestamp);
        db.expect_increment_storage()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts| ts == &timestamp)
            .return_once(|_, _| Ok(()));
        db.expect_fetch_timestamp_messages()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts, _| ts == &Some(timestamp))
            .return_once(|_, _, _| Ok(FetchMessageResponse::default()));
        let (app_state, packets) = spy_app_state(db);
        let delivery_metrics = app_state.delivery_metrics.clone();
        let (mut client, _) = wpclient(DUMMY_UAID, app_state).await;

        client
            .on_server_notif(ServerNotification::CheckStorage)
            .await
            .unwrap();
        client.on_client_msg(ack("v1")).await.unwrap();
        // Nothing is counted until the whole batch is Ack'd
        assert_eq!(outcome_count(&packets(), "delivered"), 0);
        client.on_client_msg(ack("v2")).await.unwrap();

        let packets = packets();
        assert_eq!(outcome_count(&packets, "delivered"), 2);
        assert_eq!(outcome_count(&packets, "expired"), 1);
        let report = delivery_metrics.encode().unwrap();
        assert!(report.contains(r#"autopush_delivery_seconds_count{path="ws",route="stored"} 2"#));
        assert!(report.contains("autopush_stored_delivery_after_connect_seconds_count 2"));
    }

    /// An Ack'd topic message is only counted once it's removed from storage
    #[actix_rt::test]
    async fn acked_topic_counted_after_remove() {
        let mut db = MockDbClient::new();
        db.expect_fetch_topic_messages()
            .times(1)
            .return_once(|_, _| {
                Ok(FetchMessageResponse {
                    timestamp: None,
                    messages: vec![Notification {
                        version: "topic".to_owned(),
                        ..new_topic_notif(&DUMMY_CHID, 60)
                    }],
                })
            });
        db.expect_remove_message()
            .times(1)
            .return_once(|_, _| Err(DbError::General("remove failed".to_owned())));
        let (app_state, packets) = spy_app_state(db);
        let (mut client, _) = wpclient(DUMMY_UAID, app_state).await;

        client
            .on_server_notif(ServerNotification::CheckStorage)
            .await
            .unwrap();
        client.on_client_msg(ack("topic")).await.unwrap_err();
        assert_eq!(outcome_count(&packets(), "delivered"), 0);
    }

    /// An expired topic message is only counted once it's removed from storage
    #[actix_rt::test]
    async fn expired_topic_counted_after_remove() {
        let mut db = MockDbClient::new();
        db.expect_fetch_topic_messages()
            .times(1)
            .return_once(|_, _| {
                Ok(FetchMessageResponse {
                    timestamp: None,
                    messages: vec![new_topic_notif(&DUMMY_CHID, 0)],
                })
            });
        db.expect_remove_message()
            .times(1)
            .return_once(|_, _| Err(DbError::General("remove failed".to_owned())));
        let (app_state, packets) = spy_app_state(db);
        let (mut client, _) = wpclient(DUMMY_UAID, app_state).await;

        client
            .on_server_notif(ServerNotification::CheckStorage)
            .await
            .unwrap_err();
        assert_eq!(outcome_count(&packets(), "expired"), 0);
    }

    #[actix_rt::test]
    async fn webpush_ping() {
        let (mut client, _) = wpclient(DUMMY_UAID, Default::default()).await;
        let pong = client.on_client_msg(ClientMessage::Ping).await.unwrap();
        assert!(matches!(pong.as_slice(), [ServerMessage::Ping]));
    }

    #[actix_rt::test]
    async fn direct_ack_records_delivery() {
        let app_state = AppState::default();
        let delivery_metrics = app_state.delivery_metrics.clone();
        let (mut client, _) = wpclient(DUMMY_UAID, app_state).await;
        let notif = Notification {
            version: "v1".to_owned(),
            received_ms: Some(ms_since_epoch() - 2_000),
            ..new_timestamp_notif(&DUMMY_CHID, 60)
        };
        let smsgs = client
            .on_server_notif(ServerNotification::Notification(notif))
            .await
            .unwrap();
        assert!(matches!(smsgs.as_slice(), [ServerMessage::Notification(_)]));

        client
            .on_client_msg(ClientMessage::Ack {
                updates: vec![ClientAck {
                    channel_id: DUMMY_CHID,
                    version: "v1".to_owned(),
                    code: Some(101),
                }],
            })
            .await
            .unwrap();

        let report = delivery_metrics.encode().unwrap();
        // Received 2s before the ACK
        assert!(
            report.contains(
                r#"autopush_delivery_seconds_bucket{le="1.0",path="ws",route="direct"} 0"#
            )
        );
        assert!(
            report.contains(
                r#"autopush_delivery_seconds_bucket{le="2.5",path="ws",route="direct"} 1"#
            )
        );
        // Not a stored message
        assert!(report.contains("autopush_stored_delivery_after_connect_seconds_count 0"));
    }

    #[actix_rt::test]
    async fn direct_ttl0_ack_records_delivery_and_is_not_stored() {
        let mut db = MockDbClient::new();
        // TTL=0 notifications are never saved on shutdown, whether unacked or
        // still queued
        db.expect_save_messages().never();
        let app_state = AppState {
            db: db.into_boxed_arc(),
            ..Default::default()
        };
        let delivery_metrics = app_state.delivery_metrics.clone();
        let (mut client, _) = wpclient(DUMMY_UAID, app_state).await;
        let notif = Notification {
            version: "v1".to_owned(),
            received_ms: Some(ms_since_epoch()),
            ..new_timestamp_notif(&DUMMY_CHID, 0)
        };
        client
            .on_server_notif(ServerNotification::Notification(notif))
            .await
            .unwrap();
        // A second TTL=0 notification left unacked at shutdown
        let notif = Notification {
            version: "v2".to_owned(),
            ..new_timestamp_notif(&DUMMY_CHID, 0)
        };
        client
            .on_server_notif(ServerNotification::Notification(notif))
            .await
            .unwrap();

        client
            .on_client_msg(ClientMessage::Ack {
                updates: vec![ClientAck {
                    channel_id: DUMMY_CHID,
                    version: "v1".to_owned(),
                    code: Some(100),
                }],
            })
            .await
            .unwrap();
        let report = delivery_metrics.encode().unwrap();
        assert!(report.contains(r#"autopush_delivery_seconds_count{path="ws",route="direct"} 1"#));

        // A TTL=0 notification still queued when the session ends
        client.on_server_notif_shutdown(ServerNotification::Notification(Notification {
            version: "v3".to_owned(),
            ..new_timestamp_notif(&DUMMY_CHID, 0)
        }));
        client.shutdown(None);
    }

    /// An unacked TTL=0 notification must not stop storage processing
    #[actix_rt::test]
    async fn unacked_ttl0_does_not_block_storage() {
        let mut db = MockDbClient::new();
        let mut seq = mockall::Sequence::new();
        let timestamp = sec_since_epoch();
        let stored = Notification {
            version: "stored".to_owned(),
            ..new_timestamp_notif(&DUMMY_CHID, 60)
        };
        db.expect_fetch_topic_messages()
            .times(1)
            .in_sequence(&mut seq)
            .return_once(|_, _| Ok(FetchMessageResponse::default()));
        db.expect_fetch_timestamp_messages()
            .times(1)
            .in_sequence(&mut seq)
            .return_once(move |_, _, _| {
                Ok(FetchMessageResponse {
                    timestamp: Some(timestamp),
                    messages: vec![stored],
                })
            });
        // Acking the stored message advances storage and reads the next batch
        db.expect_increment_storage()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts| ts == &timestamp)
            .return_once(|_, _| Ok(()));
        db.expect_fetch_timestamp_messages()
            .times(1)
            .in_sequence(&mut seq)
            .withf(move |_, ts, _| ts == &Some(timestamp))
            .return_once(|_, _, _| Ok(FetchMessageResponse::default()));
        db.expect_save_messages().never();

        let (mut client, _) = wpclient(
            DUMMY_UAID,
            AppState {
                db: db.into_boxed_arc(),
                ..Default::default()
            },
        )
        .await;

        // A TTL=0 notification the client never acks
        client
            .on_server_notif(ServerNotification::Notification(Notification {
                version: "ttl0".to_owned(),
                ..new_timestamp_notif(&DUMMY_CHID, 0)
            }))
            .await
            .unwrap();
        let smsgs = client
            .on_server_notif(ServerNotification::CheckStorage)
            .await
            .unwrap();
        assert!(matches!(smsgs.as_slice(), [ServerMessage::Notification(_)]));

        client
            .on_client_msg(ClientMessage::Ack {
                updates: vec![ClientAck {
                    channel_id: DUMMY_CHID,
                    version: "stored".to_owned(),
                    code: Some(100),
                }],
            })
            .await
            .unwrap();
        client.shutdown(None);
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
