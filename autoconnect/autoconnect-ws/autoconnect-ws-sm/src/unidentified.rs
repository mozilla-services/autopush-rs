use std::{fmt, sync::Arc};

use cadence::CountedExt;
use uuid::Uuid;

use autoconnect_common::protocol::{ClientMessage, ServerMessage};
use autoconnect_settings::AppState;
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
    pub app_state: Arc<AppState>,
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
            //broadcasts,
            ..
        } = msg else {
            return Err(SMError::InvalidMessage(
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
            .map_err(|_| SMError::InvalidMessage("Invalid uaid".to_owned()))?;

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
        // TODO: broadcasts
        //let desired_broadcasts = Broadcast::from_hasmap(broadcasts.unwrap_or_default());
        //let (initialized_subs, broadcasts) = app_state.broadcast_init(&desired_broadcasts);
        let (wpclient, check_storage_smsgs) = WebPushClient::new(
            uaid,
            self.ua,
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
            // TODO:
            broadcasts: std::collections::HashMap::new(),
        };
        let smsgs = std::iter::once(smsg).chain(check_storage_smsgs);
        Ok((wpclient, smsgs))
    }

    async fn get_or_create_user(&self, uaid: Option<Uuid>) -> DbResult<GetOrCreateUser> {
        trace!("❓UnidentifiedClient::get_or_create_user");
        let connected_at = ms_since_epoch();

        if let Some(uaid) = uaid {
            // NOTE: previously a user would be dropped when
            // serde_dynamodb::from_hashmap failed (but this now occurs inside
            // the db impl)
            let maybe_user = self.app_state.db.get_user(&uaid).await?;
            if let Some(user) = maybe_user {
                if let Some((mut user, flags)) =
                    process_existing_user(&self.app_state, user).await?
                {
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
            // NOTE: when the client specified a uaid and get_user returns None
            // (or process_existing_user dropped the user record because it was
            // invalid) we're now deferring registration (this is change from
            // the previous state machine impl)
        }

        // TODO: NOTE: A new User doesn't get a `set_last_connect()` (matching
        // the previous impl)
        let user = User {
            current_month: Some(self.app_state.db.message_table().to_owned()),
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

    use crate::error::SMError;

    use super::UnidentifiedClient;

    fn uclient(app_state: AppState) -> UnidentifiedClient {
        UnidentifiedClient::new(UA.to_owned(), Arc::new(app_state))
    }

    #[tokio::test]
    async fn reject_not_hello() {
        let client = uclient(Default::default());
        let result = client.on_client_msg(ClientMessage::Ping).await;
        assert!(matches!(result, Err(SMError::InvalidMessage(_))));

        let client = uclient(Default::default());
        let result = client
            .on_client_msg(ClientMessage::Register {
                channel_id: DUMMY_CHID.to_string(),
                key: None,
            })
            .await;
        assert!(matches!(result, Err(SMError::InvalidMessage(_))));
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
