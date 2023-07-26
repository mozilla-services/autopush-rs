use std::{collections::HashMap, fmt, sync::Arc};

use cadence::CountedExt;
use uuid::Uuid;

use autoconnect_common::{
    broadcast::{Broadcast, BroadcastSubs, BroadcastSubsInit},
    protocol::{BroadcastValue, ClientMessage, ServerMessage},
};
use autoconnect_settings::{AppState, Settings};
use autopush_common::{
    db::{error::DbResult, User},
    util::ms_since_epoch,
};

use crate::{
    error::SMError,
    identified::{process_existing_user, ClientFlags, WebPushClient},
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
        let ClientMessage::Hello {
            uaid,
            use_webpush: Some(true),
            broadcasts,
            ..
        } = msg else {
            return Err(SMError::invalid_message(
                r#"Expected messageType="hello", "use_webpush": true"#.to_owned()
            ));
        };
        debug!(
            "👋UnidentifiedClient::on_client_msg Hello from uaid?: {:?}",
            uaid
        );

        let uaid = uaid
            .as_deref()
            .map(Uuid::try_parse)
            .transpose()
            .map_err(|_| SMError::invalid_message("Invalid uaid".to_owned()))?;

        let GetOrCreateUser {
            user,
            existing_user,
            flags,
        } = self.get_or_create_user(uaid).await?;
        let uaid = user.uaid;
        debug!(
            "💬UnidentifiedClient::on_client_msg Hello! uaid: {} existing_user: {}",
            uaid, existing_user,
        );
        let _ = self.app_state.metrics.incr("ua.command.hello");

        let (broadcast_subs, broadcasts) = self
            .broadcast_init(&Broadcast::from_hashmap(broadcasts.unwrap_or_default()))
            .await;
        let (wpclient, check_storage_smsgs) = WebPushClient::new(
            uaid,
            self.ua,
            broadcast_subs,
            flags,
            user.connected_at,
            (!existing_user).then_some(user),
            self.app_state,
        )
        .await?;

        let smsg = ServerMessage::Hello {
            uaid: uaid.as_simple().to_string(),
            status: 200,
            use_webpush: Some(true),
            broadcasts,
        };
        let smsgs = std::iter::once(smsg).chain(check_storage_smsgs);
        Ok((wpclient, smsgs))
    }

    /// Lookup a User or return a new User record if the lookup failed
    async fn get_or_create_user(&self, uaid: Option<Uuid>) -> DbResult<GetOrCreateUser> {
        trace!("❓UnidentifiedClient::get_or_create_user");
        let connected_at = ms_since_epoch();

        if let Some(uaid) = uaid {
            // NOTE: previously a user would be dropped when
            // serde_dynamodb::from_hashmap failed (but this now occurs inside
            // the db impl)
            if let Some(mut user) = self.app_state.db.get_user(&uaid).await? {
                if let Some(flags) = process_existing_user(&self.app_state, &user).await? {
                    user.node_id = Some(self.app_state.router_url.to_owned());
                    user.connected_at = connected_at;
                    user.set_last_connect();
                    self.app_state.db.update_user(&user).await?;
                    return Ok(GetOrCreateUser {
                        user,
                        existing_user: true,
                        flags,
                    });
                }
            }
            // NOTE: when the client's specified a uaid but get_user returns
            // None (or process_existing_user dropped the user record due to it
            // being invalid) we're now deferring the db.add_user call (a
            // change from the previous state machine impl)
        }

        // TODO: NOTE: A new User doesn't get a `set_last_connect()` (matching
        // the previous impl)
        let user = User {
            current_month: self
                .app_state
                .db
                .rotating_message_table()
                .map(str::to_owned),
            node_id: Some(self.app_state.router_url.to_owned()),
            connected_at,
            ..Default::default()
        };
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
        trace!("UnidentifiedClient::broadcast_init");
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
    use std::sync::Arc;

    use autoconnect_common::{
        protocol::ClientMessage,
        test_support::{hello_again_db, hello_db, DUMMY_CHID, DUMMY_UAID, UA},
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
        let client = uclient(Default::default());
        let err = client
            .on_client_msg(ClientMessage::Ping)
            .await
            .err()
            .unwrap();
        assert!(matches!(err.kind, SMErrorKind::InvalidMessage(_)));

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
    }

    #[tokio::test]
    async fn hello_existing_user() {
        let client = uclient(AppState {
            db: hello_again_db(DUMMY_UAID).into_boxed_arc(),
            ..Default::default()
        });
        let msg = ClientMessage::Hello {
            uaid: Some(DUMMY_UAID.to_string()),
            channel_ids: None,
            use_webpush: Some(true),
            broadcasts: None,
        };
        client.on_client_msg(msg).await.expect("Hello failed");
    }

    #[tokio::test]
    async fn hello_new_user() {
        let client = uclient(AppState {
            // Simple hello_db ensures no writes to the db
            db: hello_db().into_boxed_arc(),
            ..Default::default()
        });
        let msg = ClientMessage::Hello {
            uaid: None,
            channel_ids: None,
            use_webpush: Some(true),
            broadcasts: None,
        };
        client.on_client_msg(msg).await.expect("Hello failed");
    }

    #[tokio::test]
    async fn hello_bad_user() {}
}
