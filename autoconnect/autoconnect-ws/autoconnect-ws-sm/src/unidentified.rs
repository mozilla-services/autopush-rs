use std::{collections::HashMap, fmt, sync::Arc};

use cadence::Histogrammed;
use uuid::Uuid;

use autoconnect_common::{
    broadcast::{Broadcast, BroadcastSubs, BroadcastSubsInit},
    protocol::{BroadcastValue, ClientMessage, MessageType, ServerMessage},
};
use autoconnect_settings::{AppState, Settings};
use autopush_common::{
    db::{User, USER_RECORD_VERSION},
    metric_name::MetricName,
    metrics::StatsdClientExt,
    util::{ms_since_epoch, ms_utc_midnight},
};

use crate::{
    error::{SMError, SMErrorKind},
    identified::{ClientFlags, WebPushClient},
};

/// Represents a Client waiting for (or yet to process) a Hello message
/// identifying itself
pub struct UnidentifiedClient {
    /// Client's User-Agent header
    ua: String,
    app_state: Arc<AppState>,
}

impl fmt::Debug for UnidentifiedClient {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("UnidentifiedClient")
            .field("ua", &self.ua)
            .finish()
    }
}

impl UnidentifiedClient {
    pub fn new(ua: String, app_state: Arc<AppState>) -> Self {
        UnidentifiedClient { ua, app_state }
    }

    /// Return a reference to `AppState`'s `Settings`
    pub fn app_settings(&self) -> &Settings {
        &self.app_state.settings
    }

    /// Handle a WebPush `ClientMessage` sent from the user agent over the
    /// WebSocket for this user
    ///
    /// Anything but a Hello message is rejected as an Error
    pub async fn on_client_msg(
        self,
        msg: ClientMessage,
    ) -> Result<(WebPushClient, impl IntoIterator<Item = ServerMessage>), SMError> {
        trace!("❓UnidentifiedClient::on_client_msg");
        // Validate we received a Hello message before proceeding
        SMError::validate_message_type(MessageType::Hello, &msg)?;

        // Extract fields from the Hello message
        let ClientMessage::Hello {
            uaid,
            broadcasts,
            _channel_ids,
        } = msg
        else {
            // This should never happen due to the validate_message_type check above
            return Err(SMError::expected_message_type(MessageType::Hello));
        };
        debug!(
            "👋UnidentifiedClient::on_client_msg {} from uaid?: {:?}",
            MessageType::Hello.as_ref(),
            uaid
        );

        // Ignore invalid uaids (treat as None) so they'll be issued a new one
        let original_uaid = uaid.as_deref().and_then(|uaid| Uuid::try_parse(uaid).ok());

        let GetOrCreateUser {
            user,
            existing_user,
            flags,
        } = self.get_or_create_user(original_uaid).await?;
        let uaid = user.uaid;
        debug!(
            "💬UnidentifiedClient::on_client_msg {}! uaid: {} existing_user: {}",
            MessageType::Hello.as_ref(),
            uaid,
            existing_user,
        );
        self.app_state
            .metrics
            .incr_with_tags(MetricName::UaCommand(MessageType::Hello.to_string()))
            .with_tag("uaid", {
                if existing_user {
                    "existing"
                } else if original_uaid.unwrap_or(uaid) != uaid {
                    "reassigned"
                } else {
                    "new"
                }
            })
            .send();

        // This is the first time that the user has connected "today".
        if flags.emit_channel_metrics {
            // Return the number of channels for the user using the internal channel_count.
            // NOTE: this metric can be approximate since we're sampling to determine the
            // approximate average of channels per user for business reasons.
            self.app_state
                .metrics
                .histogram_with_tags("ua.connection.channel_count", user.channel_count() as u64)
                .with_tag_value("desktop")
                .send();
        }

        let (broadcast_subs, broadcasts) = self
            .broadcast_init(&Broadcast::from_hashmap(broadcasts.unwrap_or_default()))
            .await;
        let (wpclient, check_storage_smsgs) = WebPushClient::new(
            uaid,
            self.ua,
            broadcast_subs,
            flags,
            user.connected_at,
            user.current_timestamp,
            (!existing_user).then_some(user),
            self.app_state,
        )
        .await?;

        let smsg = ServerMessage::Hello {
            uaid: uaid.as_simple().to_string(),
            use_webpush: true,
            status: 200,
            broadcasts,
        };
        let smsgs = std::iter::once(smsg).chain(check_storage_smsgs);
        Ok((wpclient, smsgs))
    }

    /// Lookup a User or return a new User record if the lookup failed
    async fn get_or_create_user(&self, uaid: Option<Uuid>) -> Result<GetOrCreateUser, SMError> {
        trace!(
            "❓UnidentifiedClient::get_or_create_user for {}",
            MessageType::Hello.as_ref()
        );
        let connected_at = ms_since_epoch();

        if let Some(uaid) = uaid {
            if let Some(mut user) = self.app_state.db.get_user(&uaid).await? {
                let flags = ClientFlags {
                    check_storage: true,
                    old_record_version: user
                        .record_version
                        .is_none_or(|rec_ver| rec_ver < USER_RECORD_VERSION),
                    emit_channel_metrics: user.connected_at < ms_utc_midnight(),
                    ..Default::default()
                };
                user.node_id = Some(self.app_state.router_url.to_owned());
                if user.connected_at > connected_at {
                    let _ = self.app_state.metrics.incr(MetricName::UaAlreadyConnected);
                    return Err(SMErrorKind::AlreadyConnected.into());
                }
                user.connected_at = connected_at;
                if !self.app_state.db.update_user(&mut user).await? {
                    let _ = self.app_state.metrics.incr(MetricName::UaAlreadyConnected);
                    return Err(SMErrorKind::AlreadyConnected.into());
                }
                return Ok(GetOrCreateUser {
                    user,
                    existing_user: true,
                    flags,
                });
            }
            // NOTE: when the client's specified a uaid but get_user returns
            // None (or process_existing_user dropped the user record due to it
            // being invalid) we're now deferring the db.add_user call (a
            // change from the previous state machine impl)
        }

        let user = User::builder()
            .node_id(self.app_state.router_url.to_owned())
            .connected_at(connected_at)
            .build()
            .map_err(|e| SMErrorKind::Internal(format!("User::builder error: {e}")))?;
        Ok(GetOrCreateUser {
            user,
            existing_user: false,
            flags: Default::default(),
        })
    }

    /// Initialize `Broadcast`s for a new `WebPushClient`
    async fn broadcast_init(
        &self,
        broadcasts: &[Broadcast],
    ) -> (BroadcastSubs, HashMap<String, BroadcastValue>) {
        trace!(
            "UnidentifiedClient::broadcast_init for {}",
            MessageType::Hello.as_ref()
        );
        let bc = self.app_state.broadcaster.read().await;
        let BroadcastSubsInit(broadcast_subs, delta) = bc.broadcast_delta(broadcasts);
        let mut response = Broadcast::vec_into_hashmap(delta);
        let missing = bc.missing_broadcasts(broadcasts);
        if !missing.is_empty() {
            response.insert(
                "errors".to_owned(),
                BroadcastValue::Nested(Broadcast::vec_into_hashmap(missing)),
            );
        }
        (broadcast_subs, response)
    }
}

/// Result of a User lookup for a Hello message
struct GetOrCreateUser {
    user: User,
    existing_user: bool,
    flags: ClientFlags,
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;
    use std::sync::Arc;

    use autoconnect_common::{
        protocol::{ClientMessage, MessageType},
        test_support::{hello_again_db, hello_again_json, hello_db, DUMMY_CHID, DUMMY_UAID, UA},
    };
    use autoconnect_settings::AppState;

    use crate::error::SMErrorKind;

    use super::UnidentifiedClient;

    #[ctor::ctor]
    fn init_test_logging() {
        autopush_common::logging::init_test_logging();
    }

    fn uclient(app_state: AppState) -> UnidentifiedClient {
        UnidentifiedClient::new(UA.to_owned(), Arc::new(app_state))
    }

    #[tokio::test]
    async fn reject_not_hello() {
        // Test with Ping message
        let client = uclient(Default::default());
        let err = client
            .on_client_msg(ClientMessage::Ping)
            .await
            .err()
            .unwrap();
        assert!(matches!(err.kind, SMErrorKind::InvalidMessage(_)));
        // Verify error message contains expected message type
        assert!(format!("{}", err).contains(MessageType::Hello.as_ref()));

        // Test with Register message
        let client = uclient(Default::default());
        let err = client
            .on_client_msg(ClientMessage::Register {
                channel_id: DUMMY_CHID.to_string(),
                key: None,
            })
            .await
            .err()
            .unwrap();
        assert!(matches!(err.kind, SMErrorKind::InvalidMessage(_)));
        // Verify error message contains expected message type
        assert!(format!("{}", err).contains(MessageType::Hello.as_ref()));
    }

    #[tokio::test]
    async fn hello_existing_user() {
        let client = uclient(AppState {
            db: hello_again_db(DUMMY_UAID).into_boxed_arc(),
            ..Default::default()
        });
        // Use hello_again_json helper which properly uses MessageType enum
        let js = hello_again_json();
        let msg: ClientMessage = serde_json::from_str(&js).unwrap();
        client.on_client_msg(msg).await.expect("Hello failed");
    }

    #[tokio::test]
    async fn hello_new_user() {
        let client = uclient(AppState {
            // Simple hello_db ensures no writes to the db
            db: hello_db().into_boxed_arc(),
            ..Default::default()
        });
        // Ensure that we do not need to pass the "use_webpush" flag.
        // (yes, this could just be passing the string, but I want to be
        // very explicit here.)
        let json = serde_json::json!({"messageType":"hello"});
        let raw = json.to_string();
        let msg = ClientMessage::from_str(&raw).unwrap();
        client.on_client_msg(msg).await.expect("Hello failed");
    }

    #[tokio::test]
    async fn hello_empty_uaid() {
        let client = uclient(Default::default());
        let msg = ClientMessage::Hello {
            uaid: Some("".to_owned()),
            _channel_ids: None,
            broadcasts: None,
        };
        client.on_client_msg(msg).await.expect("Hello failed");
    }

    #[tokio::test]
    async fn hello_invalid_uaid() {
        let client = uclient(Default::default());
        let msg = ClientMessage::Hello {
            uaid: Some("invalid".to_owned()),
            _channel_ids: None,
            broadcasts: None,
        };
        client.on_client_msg(msg).await.expect("Hello failed");
    }

    #[tokio::test]
    async fn hello_bad_user() {}
}
