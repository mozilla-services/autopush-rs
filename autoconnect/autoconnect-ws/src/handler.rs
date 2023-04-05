use std::sync::Arc;

use actix_ws::{CloseReason, Message};
use futures::{channel::mpsc, stream::StreamExt};
use tokio::{
    select,
    time::{interval, timeout},
};

use autoconnect_common::protocol::{ServerMessage, ServerNotification};
use autoconnect_settings::AppState;
use autoconnect_ws_sm::{UnidentifiedClient, WebPushClient};

use crate::{
    error::WSError,
    session::{Session, SessionImpl},
};

/// WebPush WebSocket handler Task
pub async fn webpush_ws(
    session: actix_ws::Session,
    msg_stream: actix_ws::MessageStream,
    app_state: Arc<AppState>,
    ua: String,
) {
    let client = UnidentifiedClient::new(ua, app_state);
    let mut session = SessionImpl::new(session.clone());
    let close_reason = _webpush_ws(client, &mut session, msg_stream)
        .await
        .unwrap_or_else(|e| {
            Some(CloseReason {
                code: e.close_code(),
                description: Some(e.to_string()),
            })
        });
    let _ = session.close(close_reason).await;
}

/// WebPush WebSocket handler
///
/// Transistions the client between the `UnidentifiedClient` (waiting for a
/// Hello) state and the identified full `WebPushClient`.
///
/// Manages:
///
///- I/O to and from the client: incoming `ClientMessage`s from the WebSocket
/// connection and `ServerNotification`s from the `ClientRegistry` (triggered
/// by `autoendpoint`), and in turn outgoing `ServerMessage`s written to the
/// WebSocket connection in response to those events.
/// - the lifecycle/cleanup of the Client
async fn _webpush_ws(
    client: UnidentifiedClient,
    session: &mut impl Session,
    mut msg_stream: impl futures::Stream<Item = Result<actix_ws::Message, actix_ws::ProtocolError>>
        + Unpin,
) -> Result<Option<CloseReason>, WSError> {
    let (mut client, smsgs) = unidentified_ws(client, &mut msg_stream).await?;

    // Client now identified: add them to the registry to recieve ServerNotifications
    let (registry_tx, mut snotifs_stream) = mpsc::unbounded();
    // TODO: should return a scopeguard to ensure disconnect() always called
    client
        .app_state
        .clients
        .connect(client.uaid, client.uid, registry_tx)
        .await;

    // Then send their Hello response and any initial notifications from storage
    for smsg in smsgs {
        trace!(
            "_webpush_ws: New WebPushClient, ServerMessage -> session: {:#?}",
            smsg
        );
        // TODO: Ensure these added to "unacked_stored_notifs"
        session
            .text(smsg)
            .await
            // TODO: try! (?) here dictates a scopeguard for the clients entry
            .map_err(|_| WSError::StreamClosed)?;
    }

    let result = identified_ws(&mut client, session, &mut msg_stream, &mut snotifs_stream).await;

    client
        .app_state
        .clients
        .disconnect(&client.uaid, &client.uid)
        .await
        .map_err(|_| WSError::RegistryNotConnected)?;
    snotifs_stream.close();
    while let Some(snotif) = snotifs_stream.next().await {
        client.on_server_notif_shutdown(snotif);
    }
    client.shutdown();

    // TODO: report errors to sentry

    result
}

/// `UnidentifiedClient` handler
///
/// Simply waits a duration of `open_handshake_timeout` for a Hello and returns
/// an identified `WebPushClient` on success.
async fn unidentified_ws(
    client: UnidentifiedClient,
    msg_stream: &mut (impl futures::Stream<Item = Result<actix_ws::Message, actix_ws::ProtocolError>>
              + Unpin),
) -> Result<(WebPushClient, impl IntoIterator<Item = ServerMessage>), WSError> {
    let stream_with_timeout = timeout(
        client.app_state.settings.open_handshake_timeout,
        msg_stream.next(),
    );
    let msg = match stream_with_timeout.await {
        Ok(Some(msg)) => msg?,
        Ok(None) => return Err(WSError::StreamClosed),
        Err(_) => return Err(WSError::HandshakeTimeout),
    };
    trace!("unidentified_ws: Initial msg: {:#?}", msg);

    let client_msg = match msg {
        Message::Text(ref bytestring) => bytestring.parse()?,
        _ => {
            return Err(WSError::UnsupportedMessage("Expected Text".to_owned()));
        }
    };

    Ok(client.on_client_msg(client_msg).await?)
}

/// The identified `WebPushClient` handler
/// TODO: docs
async fn identified_ws(
    client: &mut WebPushClient,
    session: &mut impl Session,
    msg_stream: &mut (impl futures::Stream<Item = Result<actix_ws::Message, actix_ws::ProtocolError>>
              + Unpin),
    snotifs_stream: &mut mpsc::UnboundedReceiver<ServerNotification>,
) -> Result<Option<CloseReason>, WSError> {
    dbg!(client.app_state.settings.auto_ping_interval);
    let mut ping_interval = interval(client.app_state.settings.auto_ping_interval);
    ping_interval.tick().await;
    let close_reason = loop {
        select! {
            maybe_result = msg_stream.next() => {
                let Some(result) = maybe_result else {
                    trace!("identified_ws: msg_stream EOF");
                    // End of stream
                    break None;
                };
                let msg = result?;
                trace!("identified_ws: msg: {:#?}", msg);
                let client_msg = match msg {
                    Message::Text(ref bytestring) => bytestring.parse()?,
                    Message::Nop => continue,
                    Message::Close(reason) => break reason,
                    Message::Ping(bytes) => {
                        session.pong(&bytes).await?;
                        continue;
                    },
                    Message::Pong(_) => {
                        // TODO: A fully working "PingManager" should track
                        // last pong recieved, etc
                        ping_interval.reset();
                        continue;
                    },
                    _ => return Err(WSError::UnsupportedMessage("Expected Text, etc.".to_owned()))
                };
                for smsg in client.on_client_msg(client_msg).await? {
                    trace!("identified_ws: msg_stream, ServerMessage -> session {:#?}", smsg);
                    session
                        .text(smsg)
                        .await
                        .map_err(|_| WSError::StreamClosed)?;
                }
            },

            maybe_snotif = snotifs_stream.next() => {
                let Some(snotif) = maybe_snotif else {
                    trace!("identified_ws: snotifs_stream EOF");
                    return Err(WSError::RegistryDisconnected);
                };
                for smsg in client.on_server_notif(snotif).await? {
                    trace!("identified_ws: snotifs_stream, ServerMessage -> session {:#?}", smsg);
                    session
                        .text(smsg)
                        .await
                        .map_err(|_| WSError::StreamClosed)?;
                }
            }

            _ = ping_interval.tick() => {
                trace!("identified_ws: ping_interval tick");
                // TODO: A fully working "PingManager"
                session.ping(&[]).await?;
            }
        }
    };

    Ok(close_reason)
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use async_stream::stream;
    use futures::pin_mut;

    use autoconnect_common::{
        protocol::ServerMessage,
        test_support::{hello_db, HELLO, UA},
    };
    use autoconnect_settings::{AppState, Settings};
    use autoconnect_ws_sm::UnidentifiedClient;

    use crate::{error::WSError, session::MockSession};

    use super::_webpush_ws;

    fn uclient(app_state: AppState) -> UnidentifiedClient {
        UnidentifiedClient::new(UA.to_owned(), Arc::new(app_state))
    }

    #[actix_web::test]
    async fn handshake_timeout() {
        let settings = Settings {
            open_handshake_timeout: Duration::from_secs_f32(0.25),
            ..Default::default()
        };
        let client = uclient(AppState::from_settings(settings).unwrap());

        let s = stream! {
            tokio::time::sleep(Duration::from_secs_f32(0.3)).await;
            yield Ok(actix_ws::Message::Text(HELLO.into()));
        };
        pin_mut!(s);
        let err = _webpush_ws(client, &mut MockSession::new(), s)
            .await
            .unwrap_err();
        assert!(matches!(err, WSError::HandshakeTimeout));
    }

    #[actix_web::test]
    async fn basic() {
        let client = uclient(AppState {
            db: hello_db().into_boxed_arc(),
            ..Default::default()
        });
        let mut session = MockSession::new();
        session
            .expect_text()
            .withf(|msg| matches!(msg, ServerMessage::Hello { .. }))
            .times(1)
            .return_once(|_| Ok(()));
        session.expect_ping().never();

        let s = futures::stream::iter(vec![
            Ok(actix_ws::Message::Text(HELLO.into())),
            Ok(actix_ws::Message::Nop),
        ]);
        _webpush_ws(client, &mut session, s)
            .await
            .expect("Handler failed");
    }
    #[actix_web::test]
    async fn websocket_ping() {
        let settings = Settings {
            auto_ping_interval: Duration::from_secs_f32(0.25),
            ..Default::default()
        };
        let client = uclient(AppState {
            db: hello_db().into_boxed_arc(),
            ..AppState::from_settings(settings).unwrap()
        });
        let mut session = MockSession::new();
        session.expect_text().times(1).return_once(|_| Ok(()));
        session.expect_ping().times(1).return_once(|_| Ok(()));

        let s = stream! {
            yield Ok(actix_ws::Message::Text(HELLO.into()));
            tokio::time::sleep(Duration::from_secs_f32(0.3)).await;
        };
        pin_mut!(s);
        _webpush_ws(client, &mut session, s)
            .await
            .expect("Handler failed");
    }
}
