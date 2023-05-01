use std::{fmt, sync::Arc};

use cadence::CountedExt;

use autoconnect_common::protocol::{ClientMessage, ServerMessage};
use autoconnect_settings::AppState;
use autopush_common::util::ms_since_epoch;

use crate::{
    error::SMError,
    identified::{ClientFlags, WebPushClient},
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

        let defer_registration = uaid.is_none();
        let hello_response = self
            .app_state
            .db
            .hello(
                connected_at,
                uaid.as_ref(),
                &self.app_state.router_url,
                defer_registration,
            )
            .await?;
        trace!(
            "💬UnidentifiedClient::on_client_msg Hello! uaid: {:?} user_is_registered: {}",
            hello_response.uaid,
            hello_response.deferred_user_registration.is_none()
        );

        let Some(uaid) = hello_response.uaid else {
            trace!("💬UnidentifiedClient::on_client_msg AlreadyConnected {:?}", hello_response.uaid);
            return Err(SMError::AlreadyConnected);
        };

        let _ = self.app_state.metrics.incr("ua.command.hello");
        // TODO: broadcasts
        //let desired_broadcasts = Broadcast::from_hasmap(broadcasts.unwrap_or_default());
        //let (initialized_subs, broadcasts) = app_state.broadcast_init(&desired_broadcasts);
        let (wpclient, check_storage_smsgs) = WebPushClient::new(
            uaid,
            self.ua,
            ClientFlags::from_hello(&hello_response),
            connected_at,
            hello_response.deferred_user_registration,
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
}

use uuid::Uuid;
use autopush_common::db::{HelloResponse, error::DbResult};
fn hello(client: &UnidentifiedClient,
        connected_at: u64,
        uaid: Option<&Uuid>,
        router_url: &str,
) -> DbResult<HelloResponse> {
    trace!("hello🧑🏼 uaid {:?}", &uaid);
    if let Some(uaid) = uaid {
        let user = self.get_user(uaid).await;
        match user {
            Some(user) => {
            }
        }
    }
    panic!();
}


#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use autoconnect_common::{
        protocol::ClientMessage,
        test_support::{hello_again_db, DUMMY_CHID, DUMMY_UAID, UA},
    };
    use autoconnect_settings::AppState;
    use autopush_common::db::{mock::MockDbClient, HelloResponse, User};

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
        let mut db = MockDbClient::new();
        // Ensure no write to the db
        db.expect_hello()
            .withf(|_, _, _, defer_registration| defer_registration == &true)
            .return_once(move |_, _, _, _| {
                Ok(HelloResponse {
                    uaid: Some(DUMMY_UAID),
                    deferred_user_registration: Some(User {
                        uaid: DUMMY_UAID,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
            });
        let client = uclient(AppState {
            db: db.into_boxed_arc(),
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
