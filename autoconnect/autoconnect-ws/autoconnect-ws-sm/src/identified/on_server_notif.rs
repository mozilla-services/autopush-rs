use actix_web::rt;

use autoconnect_common::protocol::{ServerMessage, ServerNotification};
use autopush_common::{
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
            ServerNotification::CheckStorage => self.check_storage().await,
            ServerNotification::Notification(notif) => Ok(vec![self.notif(notif).await?]),
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

    pub(super) async fn check_storage(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        self.flags.check_storage = true;
        self.flags.include_topic = true;
        // XXX: what about
        //while self.flags.check_storage, do it (checking limit?), extending a vec. when we get 0 from do_check, we set check_storage to false?
        self.do_check_storage().await
    }

    pub(super) async fn do_check_storage(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        eprintln!(
            "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ {}",
            self.flags.increment_storage
        );
        trace!("WebPushClient::check_storage");
        // TODO:

        // This is responsible for handling sent_from_storage > msg_limit: this
        // is tricky because historically we've sent out the check_storage
        // batch of messages before actually dropping the user. (both in the
        // python/previous state machine). Since we're dropping their messages
        // anyway, we should just drop before sending these messages. This
        // simply means we might enforce the limit at 90 (100-10) instead of
        // 100. we could also increase the limit to match the older behavior.
        //
        let CheckStorageResponse {
            include_topic,
            mut messages,
            timestamp,
        } = check_storage(self).await?;

        // XXX:
        debug!(
            "WebPushClient::check_storage include_topic -> {} unacked_stored_highest -> {:?}",
            include_topic, timestamp
        );
        self.flags.include_topic = include_topic;
        self.ack_state.unacked_stored_highest = timestamp;
        if messages.is_empty() {
            debug!("WebPushClient::check_storage finished");
            self.flags.check_storage = false;
            self.sent_from_storage = 0;
            // No need to increment_storage
            debug_assert!(!self.flags.increment_storage);
            // XXX: technically back to determine ack? (maybe_post_process_acks)?
            // XXX: DetermineAck
            // XXX: if all we need to do is increment storage, just
            // increment it right here. can't we be done then? and no
            // DeterimeAck needed (only for init but that can be
            // handled elsewhere probably..)
            // XXX: if this actually needs to set check_storage to false then DetermineAck doesn't do shit anyway..?
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
            // XXX: DetermineAck
            // XXX: see above notes
            if self.flags.increment_storage {
                self.increment_storage().await?;
            }
            self.flags.check_storage = false;
            self.sent_from_storage = 0;
            return Ok(vec![]);
        }
        self.ack_state
            .unacked_stored_notifs
            .extend(messages.iter().cloned());
        let smsgs: Vec<_> = messages
            .into_iter()
            .inspect(|msg| {
                trace!("WebPushClient::check_storage Sending stored");
                emit_metrics_for_send(&self.app_state.metrics, msg, "Stored", &self.ua_info)
            })
            .map(ServerMessage::Notification)
            .collect();
        // XXX: if less info needed could be metrics.count(.., smsgs.len());
        self.sent_from_storage += smsgs.len() as u32;
        Ok(smsgs)
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

    pub(super) async fn increment_storage(&mut self) -> Result<(), SMError> {
        let timestamp = self
            .ack_state
            .unacked_stored_highest
            .ok_or_else(|| SMError::Internal("unacked_stored_highest unset".to_owned()))?;
        self.app_state
            .db
            .increment_storage(&self.uaid, timestamp)
            .await?;
        self.flags.increment_storage = false;
        // XXX: Back to DetermineAck
        // XXX: I think just calling check_storage afterwards in maybe_post_process_acks solves this..
        Ok(())
    }

    async fn notif(&mut self, notif: Notification) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient::notif Sending a direct notif");
        if notif.ttl != 0 {
            self.ack_state.unacked_direct_notifs.push(notif.clone());
        }
        emit_metrics_for_send(&self.app_state.metrics, &notif, "Direct", &self.ua_info);
        Ok(ServerMessage::Notification(notif))
    }
}

use autopush_common::db::CheckStorageResponse;
use cadence::Counted;
async fn check_storage(client: &mut WebPushClient) -> Result<CheckStorageResponse, SMError> {
    let timestamp = client.ack_state.unacked_stored_highest;
    /*
    let maybe_msgs = client.flags.include_topic.then(|| client.app_state.db.fetch_messages(&client.uaid, 11));
    let Some(msgs) if !smsgs.is_empty() = maybe_msgs else {

    }
    */
    let resp = if client.flags.include_topic {
        debug!("check_storage: fetch_messages");
        client.app_state.db.fetch_messages(&client.uaid, 11).await?
    } else {
        Default::default()
    };
    if !resp.messages.is_empty() {
        debug!("check_storage: Topic message returns: {:?}", resp.messages);
        client
            .app_state
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

    let timestamp = if client.flags.include_topic {
        resp.timestamp
    } else {
        timestamp
    };
    let resp = if resp.messages.is_empty() || resp.timestamp.is_some() {
        debug!("check_storage: fetch_timestamp_messages");
        client
            .app_state
            .db
            .fetch_timestamp_messages(&client.uaid, timestamp, 10)
            .await?
    } else {
        Default::default()
    };
    let timestamp = resp.timestamp.or(timestamp);
    client
        .app_state
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

// XXX: move elsewhere
use cadence::{CountedExt, StatsdClient};

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
