use std::collections::HashMap;

use uuid::Uuid;

use autoconnect_common::{
    broadcast::Broadcast,
    protocol::{BroadcastValue, ClientAck, ClientMessage, MessageType, ServerMessage},
};
use autopush_common::{
    endpoint::make_endpoint, metric_name::MetricName, metrics::StatsdClientExt,
    util::sec_since_epoch,
};

use super::WebPushClient;
use crate::error::{SMError, SMErrorKind};

impl WebPushClient {
    /// Handle a WebPush `ClientMessage` sent from the user agent over the
    /// WebSocket for this user
    pub async fn on_client_msg(
        &mut self,
        msg: ClientMessage,
    ) -> Result<Vec<ServerMessage>, SMError> {
        match msg {
            ClientMessage::Hello { .. } => {
                Err(SMError::invalid_message("Already Hello'd".to_owned()))
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
                self.nack(code);
                Ok(vec![])
            }
            ClientMessage::Ping => Ok(vec![self.ping()?]),
        }
    }

    /// Register a new Push subscription
    async fn register(
        &mut self,
        channel_id_str: String,
        key: Option<String>,
    ) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient:register";
               "uaid" => &self.uaid.to_string(),
               "channel_id" => &channel_id_str,
               "key" => &key,
               "message_type" => MessageType::Register.as_ref(),
        );
        let channel_id = Uuid::try_parse(&channel_id_str).map_err(|_| {
            SMError::invalid_message(format!("Invalid channelID: {channel_id_str}"))
        })?;
        if channel_id.as_hyphenated().to_string() != channel_id_str {
            return Err(SMError::invalid_message(format!(
                "Invalid UUID format, not lower-case/dashed: {channel_id}",
            )));
        }

        let (status, push_endpoint) = match self.do_register(&channel_id, key).await {
            Ok(endpoint) => {
                let _ = self.app_state.metrics.incr(MetricName::UaCommandRegister);
                self.stats.registers += 1;
                (200, endpoint)
            }
            Err(SMErrorKind::MakeEndpoint(msg)) => {
                error!("WebPushClient::register make_endpoint failed: {}", msg);
                (400, "Failed to generate endpoint".to_owned())
            }
            Err(e) => {
                error!("WebPushClient::register failed: {}", e);
                (500, "".to_owned())
            }
        };
        Ok(ServerMessage::Register {
            channel_id,
            status,
            push_endpoint,
        })
    }

    async fn do_register(
        &mut self,
        channel_id: &Uuid,
        key: Option<String>,
    ) -> Result<String, SMErrorKind> {
        if let Some(user) = &self.deferred_add_user {
            debug!(
                "💬WebPushClient::register: User not yet registered: {}",
                &user.uaid
            );
            self.app_state.db.add_user(user).await?;
            self.deferred_add_user = None;
        }

        let endpoint = make_endpoint(
            &self.uaid,
            channel_id,
            key.as_deref(),
            &self.app_state.endpoint_url,
            &self.app_state.fernet,
        )
        .map_err(SMErrorKind::MakeEndpoint)?;
        self.app_state
            .db
            .add_channel(&self.uaid, channel_id)
            .await?;
        Ok(endpoint)
    }

    /// Unregister an existing Push subscription
    async fn unregister(
        &mut self,
        channel_id: Uuid,
        code: Option<u32>,
    ) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient:unregister";
               "uaid" => &self.uaid.to_string(),
               "channel_id" => &channel_id.to_string(),
               "code" => &code,
               "message_type" => MessageType::Unregister.as_ref(),
        );
        // TODO: (copied from previous state machine) unregister should check
        // the format of channel_id like register does

        let result = self
            .app_state
            .db
            .remove_channel(&self.uaid, &channel_id)
            .await;
        let status = match result {
            Ok(_) => {
                self.app_state
                    .metrics
                    .incr_with_tags(MetricName::UaCommandUnregister)
                    .with_tag("code", &code.unwrap_or(200).to_string())
                    .send();
                self.stats.unregisters += 1;
                200
            }
            Err(e) => {
                error!("WebPushClient::unregister failed: {}", e);
                500
            }
        };
        Ok(ServerMessage::Unregister { channel_id, status })
    }

    /// Subscribe to a new set of Broadcasts
    async fn broadcast_subscribe(
        &mut self,
        broadcasts: HashMap<String, String>,
    ) -> Result<Option<ServerMessage>, SMError> {
        trace!("WebPushClient:broadcast_subscribe"; "message_type" => MessageType::BroadcastSubscribe.as_ref());
        let broadcasts = Broadcast::from_hashmap(broadcasts);
        let mut response: HashMap<String, BroadcastValue> = HashMap::new();

        let bc = self.app_state.broadcaster.read().await;
        if let Some(delta) = bc.subscribe_to_broadcasts(&mut self.broadcast_subs, &broadcasts) {
            response.extend(Broadcast::vec_into_hashmap(delta));
        };
        let missing = bc.missing_broadcasts(&broadcasts);
        if !missing.is_empty() {
            response.insert(
                "errors".to_owned(),
                BroadcastValue::Nested(Broadcast::vec_into_hashmap(missing)),
            );
        }

        Ok((!response.is_empty()).then_some(ServerMessage::Broadcast {
            broadcasts: response,
        }))
    }

    /// Acknowledge receipt of one or more Push Notifications
    async fn ack(&mut self, updates: &[ClientAck]) -> Result<Vec<ServerMessage>, SMError> {
        trace!("✅ WebPushClient:ack"; "message_type" => MessageType::Ack.as_ref());
        let _ = self.app_state.metrics.incr(MetricName::UaCommandAck);

        for notif in updates {
            // Check the list of unacked "direct" (unstored) notifications. We only want to
            // ack messages we've not yet seen and we have the right version, otherwise we could
            // have gotten an older, inaccurate ACK.
            let pos = self
                .ack_state
                .unacked_direct_notifs
                .iter()
                .position(|n| n.channel_id == notif.channel_id && n.version == notif.version);
            // We found one, so delete it from our list of unacked messages
            if let Some(pos) = pos {
                debug!("✅ Ack (Direct)";
                       "channel_id" => notif.channel_id.as_hyphenated().to_string(),
                       "version" => &notif.version
                );
                self.ack_state.unacked_direct_notifs.remove(pos);
                self.stats.direct_acked += 1;
                continue;
            };

            // Now, check the list of stored notifications
            let pos = self
                .ack_state
                .unacked_stored_notifs
                .iter()
                .position(|n| n.channel_id == notif.channel_id && n.version == notif.version);
            if let Some(pos) = pos {
                debug!(
                    "✅ Ack (Stored)";
                       "channel_id" => notif.channel_id.as_hyphenated().to_string(),
                       "version" => &notif.version,
                       "message_type" => MessageType::Ack.as_ref()
                );
                // Get the stored notification record.
                let acked_notification = &mut self.ack_state.unacked_stored_notifs[pos];
                let is_topic = acked_notification.topic.is_some();
                debug!("✅ Ack notif: {:?}", &acked_notification);
                // Only force delete Topic messages, since they don't have a timestamp.
                // Other messages persist in the database, to be, eventually, cleaned up by their
                // TTL. We will need to update the `CurrentTimestamp` field for the channel
                // record. Use that field to set the baseline timestamp for when to pull messages
                // in the future.
                if is_topic {
                    debug!(
                        "✅ WebPushClient:ack removing Stored, sort_key: {}",
                        &acked_notification.chidmessageid()
                    );
                    self.app_state
                        .db
                        .remove_message(&self.uaid, &acked_notification.chidmessageid())
                        .await?;
                    // NOTE: timestamp messages may still be in state of flux: they're not fully
                    // ack'd (removed/unable to be resurrected) until increment_storage is called,
                    // so their reliability is recorded there
                    #[cfg(feature = "reliable_report")]
                    acked_notification
                        .record_reliability(&self.app_state.reliability, notif.reliability_state())
                        .await;
                }
                let n = self.ack_state.unacked_stored_notifs.remove(pos);
                #[cfg(feature = "reliable_report")]
                if !is_topic {
                    self.ack_state.acked_stored_timestamp_notifs.push(n);
                }
                self.stats.stored_acked += 1;
                continue;
            };
        }

        if self.ack_state.unacked_notifs() {
            // Wait for the Client to Ack all notifications before further
            // processing
            Ok(vec![])
        } else {
            self.post_process_all_acked().await
        }
    }

    /// Negative Acknowledgement (a Client error occurred) of one or more Push
    /// Notifications
    fn nack(&mut self, code: Option<i32>) {
        trace!("WebPushClient:nack"; "message_type" => MessageType::Nack.as_ref());
        // only metric codes expected from the client (or 0)
        let code = code
            .and_then(|code| (301..=303).contains(&code).then_some(code))
            .unwrap_or(0);
        self.app_state
            .metrics
            .incr_with_tags(MetricName::UaCommandNak)
            .with_tag("code", &code.to_string())
            .send();
        self.stats.nacks += 1;
    }

    /// Handle a WebPush Ping
    ///
    /// Note this is the WebPush Protocol level's Ping: this differs from the
    /// lower level WebSocket Ping frame (handled by the `webpush_ws` handler).
    fn ping(&mut self) -> Result<ServerMessage, SMError> {
        trace!("WebPushClient:ping"; "message_type" => MessageType::Ping.as_ref());
        // TODO: why is this 45 vs the comment describing a minute? and 45
        // should be a setting
        // Clients shouldn't ping > than once per minute or we disconnect them
        if sec_since_epoch() - self.last_ping >= 45 {
            trace!("🏓WebPushClient Got a WebPush Ping, sending WebPush Pong");
            self.last_ping = sec_since_epoch();
            Ok(ServerMessage::Ping)
        } else {
            Err(SMErrorKind::ExcessivePing.into())
        }
    }

    /// Post process the Client succesfully Ack'ing all Push Notifications it's
    /// been sent.
    ///
    /// Notifications are read in small batches (approximately 10). We wait for
    /// the Client to Ack every Notification in that batch (invoking this
    /// method) before proceeding to read the next batch (or potential other
    /// actions such as `reset_uaid`).
    async fn post_process_all_acked(&mut self) -> Result<Vec<ServerMessage>, SMError> {
        trace!("▶️ WebPushClient:post_process_all_acked"; "message_type" => MessageType::Notification.as_ref());
        let flags = &self.flags;
        if flags.check_storage {
            if flags.increment_storage {
                debug!(
                    "▶️ WebPushClient:post_process_all_acked check_storage && increment_storage";
                    "message_type" => MessageType::Notification.as_ref()
                );
                self.increment_storage().await?;
            }

            debug!("▶️ WebPushClient:post_process_all_acked check_storage"; "message_type" => MessageType::Notification.as_ref());
            let smsgs = self.check_storage_loop().await?;
            if !smsgs.is_empty() {
                debug_assert!(self.flags.check_storage);
                // More outgoing notifications: send them out and go back to
                // waiting for the Client to Ack them all before further
                // processing
                return Ok(smsgs);
            }
            // Otherwise check_storage is finished
            debug_assert!(!self.flags.check_storage);
            debug_assert!(!self.flags.increment_storage);
        }

        // All Ack'd and finished checking/incrementing storage
        debug_assert!(!self.ack_state.unacked_notifs());
        let flags = &self.flags;
        if flags.old_record_version {
            debug!("▶️ WebPushClient:post_process_all_acked; resetting uaid"; "message_type" => MessageType::Notification.as_ref());
            self.app_state
                .metrics
                .incr_with_tags(MetricName::UaExpiration)
                .with_tag("reason", "old_record_version")
                .send();
            self.app_state.db.remove_user(&self.uaid).await?;
            Err(SMErrorKind::UaidReset.into())
        } else {
            Ok(vec![])
        }
    }
}
