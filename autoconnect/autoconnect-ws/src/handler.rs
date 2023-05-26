use std::sync::Arc;

use actix_ws::{CloseReason, Message};
use futures::{channel::mpsc, Stream, StreamExt};
use tokio::{select, time::timeout};

use autoconnect_common::protocol::{ServerMessage, ServerNotification};
use autoconnect_settings::AppState;
use autoconnect_ws_sm::{UnidentifiedClient, WebPushClient};

use crate::{
    error::WSError,
    ping::PingManager,
    session::{Session, SessionImpl},
};

type MessageStreamResult = Result<actix_ws::Message, actix_ws::ProtocolError>;

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
pub(crate) async fn webpush_ws(
    client: UnidentifiedClient,
    session: &mut impl Session,
    mut msg_stream: impl Stream<Item = MessageStreamResult> + Unpin,
) -> Result<Option<CloseReason>, WSError> {
    // NOTE: UnidentifiedClient doesn't require shutdown/cleanup, so its
    // Error's propagated. We don't propagate Errors afterwards to handle
    // shutdown/cleanup of WebPushClient
    let (mut client, smsgs) = unidentified_ws(client, &mut msg_stream).await?;

    // Client now identified: add them to the registry to recieve ServerNotifications
    let mut snotif_stream = client.registry_connect().await;
    let result = identified_ws(&mut client, smsgs, session, msg_stream, &mut snotif_stream).await;
    client.registry_disconnect().await;

    snotif_stream.close();
    while let Some(snotif) = snotif_stream.next().await {
        client.on_server_notif_shutdown(snotif);
    }
    client.shutdown(result.as_ref().err().map(|e| e.to_string()));

    if let Err(ref e) = result {
        if e.is_sentry_event() {
            sentry::capture_event(e.to_sentry_event(&client));
        }
    }
    result
}

/// `UnidentifiedClient` handler
///
/// Simply waits a duration of `open_handshake_timeout` for a Hello and returns
/// an identified `WebPushClient` on success.
async fn unidentified_ws(
    client: UnidentifiedClient,
    msg_stream: &mut (impl Stream<Item = MessageStreamResult> + Unpin),
) -> Result<(WebPushClient, impl IntoIterator<Item = ServerMessage>), WSError> {
    let stream_with_timeout = timeout(
        client.app_settings().open_handshake_timeout,
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
    smsgs: impl IntoIterator<Item = ServerMessage>,
    session: &mut impl Session,
    mut msg_stream: impl Stream<Item = MessageStreamResult> + Unpin,
    snotif_stream: &mut mpsc::UnboundedReceiver<ServerNotification>,
) -> Result<Option<CloseReason>, WSError> {
    // Send the Hello response and any initial notifications from storage
    for smsg in smsgs {
        trace!(
            "identified_ws: New WebPushClient, ServerMessage -> session: {:#?}",
            smsg
        );
        session.text(smsg).await?;
    }

    let mut ping_manager = PingManager::new(client.app_settings()).await;
    let close_reason = loop {
        select! {
            maybe_result = msg_stream.next() => {
                let Some(result) = maybe_result else {
                    trace!("identified_ws: msg_stream EOF");
                    // End of Client stream
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
                        ping_manager.on_ws_pong(client.app_settings()).await;
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

            result = ping_manager.tick() => {
                trace!("identified_ws: ping_manager tick is_ok: {}", result.is_ok());
                // Propagate PongTimeout
                result?;
                ping_manager.ws_ping_or_broadcast(client, session).await?;
            }
        }
    };

    Ok(close_reason)
}
