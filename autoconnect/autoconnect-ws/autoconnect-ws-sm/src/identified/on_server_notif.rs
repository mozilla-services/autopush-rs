use actix_web::rt;
use cadence::{Counted, CountedExt, StatsdClient};

use autoconnect_common::protocol::{ServerMessage, ServerNotification};
use autopush_common::{
    db::CheckStorageResponse,
    notification::Notification,
    util::{sec_since_epoch, user_agent::UserAgentInfo},
};

use super::WebPushClient;
use crate::error::SMError;

impl WebPushClient {
    pub async fn on_server_notif(
        &mut self,
        snotif: ServerNotification,
    ) -> Result<Vec<ServerMessage>, SMError> {
        match snotif {
            ServerNotification::Notification(notif) => Ok(vec![self.notif(notif).await?]),
            ServerNotification::CheckStorage => self.check_storage().await,
            ServerNotification::Disconnect => Err(SMError::Ghost),
        }
    }

    /// Move queued Push Notifications to unacked_direct_notifs (to be stored
    /// in the db)
    pub fn on_server_notif_shutdown(&mut self, snotif: ServerNotification) {
        if let ServerNotification::Notification(notif) = snotif {
            self.ack_state.unacked_direct_notifs.push(notif);
        }
    }

    async fn notif(&mut self, notif: Notification) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient::notif Sending a direct notif");
        if notif.ttl != 0 {
            self.ack_state.unacked_direct_notifs.push(notif.clone());
        }
        emit_metrics_for_send(&self.app_state.metrics, &notif, "Direct", &self.ua_info);
        Ok(ServerMessage::Notification(notif))
    }

    pub(super) async fn check_storage(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("WebPushClient::check_storage");
        self.flags.check_storage = true;
        self.flags.include_topic = true;
        self.check_storage_loop().await
    }

    pub(super) async fn check_storage_loop(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("WebPushClient::check_storage_loop");
        while self.flags.check_storage {
            let smsgs = self.check_storage_advance().await?;
            if !smsgs.is_empty() {
                self.check_msg_limit().await?;
                return Ok(smsgs);
            }
        }
        Ok(vec![])
    }

    async fn check_storage_advance(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("WebPushClient::check_storage_advance");
        let CheckStorageResponse {
            include_topic,
            mut messages,
            timestamp,
            //} = do_check_storage(self).await?;
        } = self.do_check_storage().await?;

        // XXX:
        debug!(
            "WebPushClient::check_storage_advance include_topic -> {} unacked_stored_highest -> {:?}",
            include_topic, timestamp
        );
        self.flags.include_topic = include_topic;
        self.ack_state.unacked_stored_highest = timestamp;
        if messages.is_empty() {
            debug!("WebPushClient::check_storage_advance finished");
            self.flags.check_storage = false;
            self.sent_from_storage = 0;
            return Ok(vec![]);
        }

        // Filter out TTL expired messages
        // XXX: could be separated out of this method?
        let now = sec_since_epoch();
        messages.retain(|n| {
            eprintln!("n: exp: {}, {:#?}", n.expired(now), n);
            if !n.expired(now) {
                return true;
            }
            // XXX: batch remove_messages would be nice?
            if n.sortkey_timestamp.is_none() {
                self.spawn_remove_message(n.sort_key());
            }
            false
        });

        self.flags.increment_storage = !include_topic && timestamp.is_some();
        // If there's still messages send them out
        if messages.is_empty() {
            return Ok(vec![]);
        }
        self.ack_state
            .unacked_stored_notifs
            .extend(messages.iter().cloned());
        let smsgs: Vec<_> = messages
            .into_iter()
            .inspect(|msg| {
                trace!("WebPushClient::check_storage_advance Sending stored");
                emit_metrics_for_send(&self.app_state.metrics, msg, "Stored", &self.ua_info)
            })
            .map(ServerMessage::Notification)
            .collect();
        // XXX: if less info needed could be metrics.count(.., smsgs.len());
        let count = smsgs.len() as u32;
        trace!(
            "WebPushClient::check_storage_advance: sent_from_storage: {}, +{}",
            self.sent_from_storage,
            count
        );
        self.sent_from_storage += count;
        Ok(smsgs)
    }

    async fn do_check_storage(&self) -> Result<CheckStorageResponse, SMError> {
        trace!("do_check_storage");
        let timestamp = self.ack_state.unacked_stored_highest;
        /*
            let maybe_msgs = self.flags.include_topic.then(|| self.app_state.db.fetch_messages(&self.uaid, 11));
            let Some(msgs) if !smsgs.is_empty() = maybe_msgs else {

        }
             */
        let resp = if self.flags.include_topic {
            debug!("check_storage: fetch_messages");
            self.app_state.db.fetch_messages(&self.uaid, 11).await?
        } else {
            Default::default()
        };
        if !resp.messages.is_empty() {
            debug!("check_storage: Topic message returns: {:?}", resp.messages);
            self.app_state
                .metrics
                .count_with_tags("notification.message.retrieved", resp.messages.len() as i64)
                .with_tag("topic", "true")
                .send();
            return Ok(CheckStorageResponse {
                include_topic: true,
                messages: resp.messages,
                timestamp: resp.timestamp,
            });
        }

        let timestamp = if self.flags.include_topic {
            resp.timestamp
        } else {
            timestamp
        };
        // is_empty implied!! always true! wtf.
        let resp = if resp.messages.is_empty() || resp.timestamp.is_some() {
            debug!("check_storage: fetch_timestamp_messages");
            self.app_state
                .db
                .fetch_timestamp_messages(&self.uaid, timestamp, 10)
                .await?
        } else {
            Default::default()
        };
        let timestamp = resp.timestamp.or(timestamp);
        self.app_state
            .metrics
            .count_with_tags("notification.message.retrieved", resp.messages.len() as i64)
            .with_tag("topic", "false")
            .send();

        Ok(CheckStorageResponse {
            include_topic: false,
            messages: resp.messages,
            timestamp,
        })
    }

    fn spawn_remove_message(&self, sort_key: String) {
        let db = self.app_state.db.clone();
        let uaid = self.uaid;
        rt::spawn(async move {
            if db.remove_message(&uaid, &sort_key).await.is_ok() {
                debug!(
                    "Deleted expired message without sortkey_timestamp, sort_key: {}",
                    sort_key
                );
            }
        });
    }

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
            // TODO: this previously closed the connection cleanly
            return Err(SMError::UaidReset);
        }
        Ok(())
    }
}

fn emit_metrics_for_send(
    metrics: &StatsdClient,
    notif: &Notification,
    source: &'static str,
    user_agent_info: &UserAgentInfo,
) {
    metrics
        .incr_with_tags("ua.notification.sent")
        .with_tag("source", source)
        .with_tag("topic", &notif.topic.is_some().to_string())
        .with_tag("os", &user_agent_info.metrics_os)
        // TODO: include `internal` if meta is set
        .send();
    metrics
        .count_with_tags(
            "ua.message_data",
            notif.data.as_ref().map_or(0, |data| data.len() as i64),
        )
        .with_tag("source", source)
        .with_tag("os", &user_agent_info.metrics_os)
        .send();
}
