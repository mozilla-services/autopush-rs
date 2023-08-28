use cadence::{Counted, CountedExt};

use autoconnect_common::protocol::{ServerMessage, ServerNotification};
use autopush_common::{
    db::CheckStorageResponse, notification::Notification, util::sec_since_epoch,
};

use super::WebPushClient;
use crate::error::{SMError, SMErrorKind};

impl WebPushClient {
    /// Handle a `ServerNotification` for this user
    ///
    /// `ServerNotification::Disconnect` is emitted by the same autoconnect
    /// node recieving it when a User has logged into that same node twice to
    /// "Ghost" (disconnect) the first user's session for its second session.
    ///
    /// Other variants are emitted by autoendpoint
    pub async fn on_server_notif(
        &mut self,
        snotif: ServerNotification,
    ) -> Result<Vec<ServerMessage>, SMError> {
        match snotif {
            ServerNotification::Notification(notif) => Ok(vec![self.notif(notif)?]),
            ServerNotification::CheckStorage => self.check_storage().await,
            ServerNotification::Disconnect => Err(SMErrorKind::Ghost.into()),
        }
    }

    /// After disconnecting from the `ClientRegistry`, moves any queued Direct
    /// Push Notifications to unacked_direct_notifs (to be stored in the db on
    /// `shutdown`)
    pub fn on_server_notif_shutdown(&mut self, snotif: ServerNotification) {
        trace!("WebPushClient::on_server_notif_shutdown");
        if let ServerNotification::Notification(notif) = snotif {
            self.ack_state.unacked_direct_notifs.push(notif);
        }
    }

    /// Send a Direct Push Notification to this user
    fn notif(&mut self, notif: Notification) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient::notif Sending a direct notif");
        if notif.ttl != 0 {
            self.ack_state.unacked_direct_notifs.push(notif.clone());
        }
        self.emit_send_metrics(&notif, "Direct");
        Ok(ServerMessage::Notification(notif))
    }

    /// Top level read of Push Notifications from storage
    ///
    /// Initializes the top level `check_storage` and `include_topic` flags and
    /// runs `check_storage_loop`
    pub(super) async fn check_storage(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("🗄️ WebPushClient::check_storage");
        self.flags.check_storage = true;
        self.flags.include_topic = true;
        self.check_storage_loop().await
    }

    /// Loop the read of Push Notifications from storage
    ///
    /// Loops until any unexpired Push Notifications are read or there's no
    /// more Notifications in storage
    pub(super) async fn check_storage_loop(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("🗄️ WebPushClient::check_storage_loop");
        while self.flags.check_storage {
            let smsgs = self.check_storage_advance().await?;
            if !smsgs.is_empty() {
                self.check_msg_limit().await?;
                return Ok(smsgs);
            }
        }
        // No more notifications (check_storage = false). Despite returning no
        // notifs we may have advanced through expired timestamp messages and
        // need to increment_storage to mark them as deleted
        if self.flags.increment_storage {
            debug!("🗄️ WebPushClient::check_storage_loop increment_storage");
            self.increment_storage().await?;
        }
        Ok(vec![])
    }

    /// Read a chunk (max count 10 returned) of Notifications from storage
    ///
    /// This filters out expired Notifications and may return an empty result
    /// set when there's still pending Notifications to be read: so it should
    /// be called in a loop to advance through all Notification records
    async fn check_storage_advance(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("🗄️ WebPushClient::check_storage_advance");
        let CheckStorageResponse {
            include_topic,
            mut messages,
            timestamp,
        } = self.do_check_storage().await?;

        debug!(
            "🗄️ WebPushClient::check_storage_advance \
                 include_topic: {} -> {} \
                 unacked_stored_highest: {:?} -> {:?}",
            self.flags.include_topic,
            include_topic,
            self.ack_state.unacked_stored_highest,
            timestamp
        );
        self.flags.include_topic = include_topic;
        self.ack_state.unacked_stored_highest = timestamp;

        if messages.is_empty() {
            trace!("🗄️ WebPushClient::check_storage_advance finished");
            self.flags.check_storage = false;
            self.sent_from_storage = 0;
            return Ok(vec![]);
        }

        // Filter out TTL expired messages
        let now_sec = sec_since_epoch();
        // Topic messages require immediate deletion from the db
        let mut expired_topic_sort_keys = vec![];
        messages.retain(|msg| {
            if !msg.expired(now_sec) {
                return true;
            }
            if msg.sortkey_timestamp.is_none() {
                expired_topic_sort_keys.push(msg.chidmessageid());
            }
            false
        });
        // TODO: A batch remove_messages would be nicer
        for sort_key in expired_topic_sort_keys {
            trace!("🉑 removing expired topic sort key: {sort_key}");
            self.app_state
                .db
                .remove_message(&self.uaid, &sort_key)
                .await?;
        }

        self.flags.increment_storage = !include_topic && timestamp.is_some();

        if messages.is_empty() {
            trace!("🗄️ WebPushClient::check_storage_advance empty response (filtered expired)");
            return Ok(vec![]);
        }

        self.ack_state
            .unacked_stored_notifs
            .extend(messages.iter().cloned());
        let smsgs: Vec<_> = messages
            .into_iter()
            .inspect(|msg| {
                trace!("🗄️ WebPushClient::check_storage_advance Sending stored");
                self.emit_send_metrics(msg, "Stored")
            })
            .map(ServerMessage::Notification)
            .collect();

        let count = smsgs.len() as u32;
        debug!(
            "🗄️ WebPushClient::check_storage_advance: sent_from_storage: {}, +{}",
            self.sent_from_storage, count
        );
        self.sent_from_storage += count;
        Ok(smsgs)
    }

    /// Read a chunk (max count 10 returned) of Notifications from storage
    ///
    /// This alternates between reading Topic Notifications and Timestamp
    /// Notifications which are stored separately in storage.
    ///
    /// Topic Messages differ in that they replace pending Notifications with
    /// new ones if they have matching Topic names. They are used when a sender
    /// desires a scenario where multiple Messages sent to an offline device
    /// result in the user only seeing the latest Message when the device comes
    /// online.
    async fn do_check_storage(&self) -> Result<CheckStorageResponse, SMError> {
        // start at the latest unacked timestamp or the previous, latest timestamp.
        let timestamp = self
            .ack_state
            .unacked_stored_highest
            .or(self.current_timestamp);
        trace!("🗄️ WebPushClient::do_check_storage {:?}", &timestamp);
        // if we're to include topic messages, do those first.
        // NOTE: DynamoDB would also wind up fetching the `current_timestamp` when pulling these,
        // Bigtable can't fetch `current_timestamp` so we can't rely on `fetch_topic_messages()`
        // returning a reasonable timestamp.
        let topic_resp = if self.flags.include_topic {
            trace!("🗄️ WebPushClient::do_check_storage: fetch_topic_messages");
            // Get the most recent max 11 messages.
            self.app_state
                .db
                .fetch_topic_messages(&self.uaid, 11)
                .await?
        } else {
            Default::default()
        };
        // if we have topic messages...
        if !topic_resp.messages.is_empty() {
            trace!(
                "🗄️ WebPushClient::do_check_storage: Topic message returns: {:#?}",
                topic_resp.messages
            );
            self.app_state
                .metrics
                .count_with_tags(
                    "notification.message.retrieved",
                    topic_resp.messages.len() as i64,
                )
                .with_tag("topic", "true")
                .send();
            return Ok(CheckStorageResponse {
                include_topic: true,
                messages: topic_resp.messages,
                timestamp: topic_resp.timestamp,
            });
        }
        // No topic messages, so carry on with normal ones, starting from the latest timestamp.
        let timestamp = if self.flags.include_topic {
            // See above, but Bigtable doesn't return the last message read timestamp when polling
            // for topic messages. Instead, we'll use the explicitly set one we store in the User
            // record and copy into the WebPushClient struct.
            topic_resp.timestamp.or(self.current_timestamp)
        } else {
            timestamp
        };
        trace!(
            "🗄️ WebPushClient::do_check_storage: fetch_timestamp_messages timestamp: {:?}",
            timestamp
        );
        let timestamp_resp = self
            .app_state
            .db
            .fetch_timestamp_messages(&self.uaid, timestamp, 10)
            .await?;
        if !timestamp_resp.messages.is_empty() {
            trace!(
                "🗄️ WebPushClient::do_check_storage: Timestamp message returns: {:#?}",
                timestamp_resp.messages
            );
            self.app_state
                .metrics
                .count_with_tags(
                    "notification.message.retrieved",
                    timestamp_resp.messages.len() as i64,
                )
                .with_tag("topic", "false")
                .send();
        }

        Ok(CheckStorageResponse {
            include_topic: false,
            messages: timestamp_resp.messages,
            // If we didn't get a timestamp off the last query, use the
            // original value if passed one
            timestamp: timestamp_resp.timestamp.or(timestamp),
        })
    }

    /// Update the user's last Message read timestamp (for timestamp Messages)
    ///
    /// Called when a Client's Ack'd all timestamp messages sent to it to move
    /// the timestamp Messages' "pointer". See
    /// `AckState::unacked_stored_highest` for further information.
    pub(super) async fn increment_storage(&mut self) -> Result<(), SMError> {
        trace!(
            "🗄️ WebPushClient::increment_storage: unacked_stored_highest: {:?}",
            self.ack_state.unacked_stored_highest
        );
        let Some(timestamp) = self.ack_state.unacked_stored_highest else {
            return Err(SMErrorKind::Internal(
                "increment_storage w/ no unacked_stored_highest".to_owned(),
            )
            .into());
        };
        self.current_timestamp = Some(timestamp);
        self.app_state
            .db
            .increment_storage(&self.uaid, timestamp)
            .await?;
        self.flags.increment_storage = false;
        Ok(())
    }

    /// Ensure this user hasn't exceeded the maximum allowed number of messages
    /// read from storage (`Settings::msg_limit`)
    ///
    /// Drops the user record and returns the `SMErrorKind::UaidReset` error if
    /// they have
    async fn check_msg_limit(&mut self) -> Result<(), SMError> {
        trace!(
            "WebPushClient::check_msg_limit: sent_from_storage: {} msg_limit: {}",
            self.sent_from_storage,
            self.app_state.settings.msg_limit
        );
        if self.sent_from_storage > self.app_state.settings.msg_limit {
            // Exceeded the max limit of stored messages: drop the user to
            // trigger a re-register
            self.app_state.db.remove_user(&self.uaid).await?;
            return Err(SMErrorKind::UaidReset.into());
        }
        Ok(())
    }

    /// Emit metrics for a Notification to be sent to the user
    fn emit_send_metrics(&self, notif: &Notification, source: &'static str) {
        let metrics = &self.app_state.metrics;
        let ua_info = &self.ua_info;
        metrics
            .incr_with_tags("ua.notification.sent")
            .with_tag("source", source)
            .with_tag("topic", &notif.topic.is_some().to_string())
            .with_tag("os", &ua_info.metrics_os)
            // TODO: include `internal` if meta is set
            .send();
        metrics
            .count_with_tags(
                "ua.message_data",
                notif.data.as_ref().map_or(0, |data| data.len() as i64),
            )
            .with_tag("source", source)
            .with_tag("os", &ua_info.metrics_os)
            .send();
    }
}
