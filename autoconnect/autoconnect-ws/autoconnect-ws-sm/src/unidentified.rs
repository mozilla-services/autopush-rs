use std::{fmt, sync::Arc};

use cadence::CountedExt;

use autoconnect_common::protocol::{ClientMessage, ServerMessage};
use autoconnect_settings::AppState;
use autopush_common::util::ms_since_epoch;

use crate::{error::SMError, identified::WebPushClient};

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
        trace!("👋UnidentifiedClient::on_client_msg Hello {:?}", uaid);

        let connected_at = ms_since_epoch();
        let uaid = uaid
            .as_deref()
            .map(uuid::Uuid::try_parse)
            .transpose()
            .map_err(|_| SMError::InvalidMessage("Invalid uaid".to_owned()))?;

        let mut user_record = None;
        if let Some(uaid) = uaid {
            let maybe_user = self.app_state.db.get_user(&uaid).await?;
            if let Some(auser) = maybe_user {
                use crate::identified::process_existing_user;
                let (mut puser, pflags) = process_existing_user(&self.app_state, auser).await?;
                // XXX: we also previously set puser.node_id = Some(router_url), why?
                puser.connected_at = connected_at;
                self.app_state.db.update_user(&puser).await?;
                user_record = Some((puser, pflags))
            }
        }
        // NOTE: when a uaid is specified and get_user returns None we're now
        // deferring registration (a change from previous versions)
        let user_is_registered = user_record.is_some();
        let (mut user, flags) = user_record.unwrap_or_default();
        if !user_is_registered {
            user.current_month = self.app_state.db.current_message_month();
            user.node_id = Some(self.app_state.router_url.to_owned());
            user.connected_at = connected_at;
        }
        // XXX: check_storage should be false when !user_is_registered (no need) (I think it is)
        // what did the old code do?

        // XXX: should check if the user_record is None
        let uaid = user.uaid;
        trace!(
            "💬UnidentifiedClient::on_client_msg Hello! uaid: {:?} user_is_registered: {}",
            uaid,
            user_is_registered,
        );

        // XXX: This used to be triggered by *any* error returned from
        // update_user. The intention was to trap the specific case of where
        // connected_at <: connected_at. Either way, the result is the same:
        // disconnect the client. We now propagate any error from update_user,
        // which disconnects them anyway, so I don't think this check is even
        // needed any longer.
        /*
        let Some(uaid) = hello_response.uaid else {
            trace!("💬UnidentifiedClient::on_client_msg AlreadyConnected {:?}", hello_response.uaid);
            return Err(SMError::AlreadyConnected);
        };
        */

        let _ = self.app_state.metrics.incr("ua.command.hello");
        // TODO: broadcasts
        //let desired_broadcasts = Broadcast::from_hasmap(broadcasts.unwrap_or_default());
        //let (initialized_subs, broadcasts) = app_state.broadcast_init(&desired_broadcasts);
        let (wpclient, check_storage_smsgs) = WebPushClient::new(
            uaid,
            self.ua,
            flags,
            connected_at,
            (!user_is_registered).then_some(user),
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

    /*
    async fn hello(&self, uaid: &Uuid) -> DbResult<(User, ClientFlags) {
        let user = self.app_state.db.get_user(uaid).await? else {
            // User doesn't exist
            return Ok((Default::default(), Default::default()));
        }
        use crate::identified::process_existing_user;
        let (mut user, flags) = process_existing_user(&self.app_state.db, user).await?;
        // XXX: we also previously set puser.node_id = Some(router_url), why?
            user.connected_at = connected_at;
            self.app_state.db.update_user(&user).await?;
            return Some((user, flags));
        }
    }
    */
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
