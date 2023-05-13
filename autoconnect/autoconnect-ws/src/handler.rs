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
pub fn spawn_webpush_ws(
    session: actix_ws::Session,
    msg_stream: actix_ws::MessageStream,
    app_state: Arc<AppState>,
    ua: String,
) {
    actix_rt::spawn(async move {
        let client = UnidentifiedClient::new(ua, app_state);
        let mut session = SessionImpl::new(session);
        let close_reason = webpush_ws(client, &mut session, msg_stream)
            .await
            .unwrap_or_else(|e| {
                error!("spawn_webpush_ws: Error: {}", e);
                Some(CloseReason {
                    code: e.close_code(),
                    description: Some(e.close_description().to_owned()),
                })
            });
        trace!("spawn_webpush_ws: close_reason: {:#?}", close_reason);
        let _ = session.close(close_reason).await;
    });
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
async fn webpush_ws(
    client: UnidentifiedClient,
    session: &mut impl Session,
    mut msg_stream: impl futures::Stream<Item = Result<actix_ws::Message, actix_ws::ProtocolError>>
        + Unpin,
) -> Result<Option<CloseReason>, WSError> {
    // NOTE: UnidentifiedClient doesn't require shutdown/cleanup, so its
    // Error's propagated. We don't propagate Errors afterwards to handle
    // shutdown/cleanup of WebPushClient
    let (mut client, smsgs) = unidentified_ws(client, &mut msg_stream).await?;

    // Client now identified: add them to the registry to recieve ServerNotifications
    let mut snotif_stream = client
        .app_state
        .clients
        .connect(client.uaid, client.uid)
        .await;

    let result = do_webpush_ws(&mut client, smsgs, session, msg_stream, &mut snotif_stream).await;

    // Ignore disconnect Errors (Client wasn't connected)
    let _ = client
        .app_state
        .clients
        .disconnect(&client.uaid, &client.uid)
        .await;

    snotif_stream.close();
    while let Some(snotif) = snotif_stream.next().await {
        client.on_server_notif_shutdown(snotif);
    }
    client.shutdown(result.as_ref().err().map_or("".to_owned(), |e| e.to_string()));

    if let Err(ref e) = result {
        if e.is_sentry_event() {
            crate::sentry::capture_error(&client, e);
        }
    }

    result
}

async fn do_webpush_ws(
    client: &mut WebPushClient,
    smsgs: impl IntoIterator<Item = ServerMessage>,
    session: &mut impl Session,
    mut msg_stream: impl futures::Stream<Item = Result<actix_ws::Message, actix_ws::ProtocolError>>
        + Unpin,
    snotif_stream: &mut mpsc::UnboundedReceiver<ServerNotification>,
) -> Result<Option<CloseReason>, WSError> {
    // XXX: could just put this into identified_ws..
    // Send the Hello response and any initial notifications from storage
    for smsg in smsgs {
        trace!(
            "webpush_ws: New WebPushClient, ServerMessage -> session: {:#?}",
            smsg
        );
        // TODO: Ensure these added to "unacked_stored_notifs"
        session.text(smsg).await?;
    }
    identified_ws(client, session, &mut msg_stream, snotif_stream).await
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
    trace!("unidentified_ws: Handshake msg: {:?}", msg);

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
    snotif_stream: &mut mpsc::UnboundedReceiver<ServerNotification>,
) -> Result<Option<CloseReason>, WSError> {
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
                    session.text(smsg).await?;
                }
            },

            maybe_snotif = snotif_stream.next() => {
                let Some(snotif) = maybe_snotif else {
                    trace!("identified_ws: snotif_stream EOF");
                    return Err(WSError::RegistryDisconnected);
                };
                for smsg in client.on_server_notif(snotif).await? {
                    trace!("identified_ws: snotif_stream, ServerMessage -> session {:#?}", smsg);
                    session.text(smsg).await?;
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

    use super::webpush_ws;

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
        let err = webpush_ws(client, &mut MockSession::new(), s)
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
        webpush_ws(client, &mut session, s)
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
        webpush_ws(client, &mut session, s)
            .await
            .expect("Handler failed");
    }
}
