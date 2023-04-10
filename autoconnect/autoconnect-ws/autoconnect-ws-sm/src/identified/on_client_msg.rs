use std::collections::HashMap;

use cadence::CountedExt;
use uuid::Uuid;

use autoconnect_common::protocol::{ClientAck, ClientMessage, ServerMessage};
use autopush_common::{endpoint::make_endpoint, util::sec_since_epoch};

use super::WebPushClient;
use crate::error::SMError;

impl WebPushClient {
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

        let smsg = match self.do_register(&channel_id, key).await {
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

    async fn do_register(
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
        // TODO: why is this 45 vs the comment describing a minute? and 45
        // should be a setting
        // Clients shouldn't ping > than once per minute or we disconnect them
        if sec_since_epoch() - self.last_ping >= 45 {
            trace!("🏓WebPushClient Got a WebPush Ping, sending WebPush Pong");
            Ok(ServerMessage::Ping)
        } else {
            Err(SMError::ExcessivePing)
        }
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
}
