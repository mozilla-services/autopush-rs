use std::{sync::Arc, time::Duration};

use async_stream::stream;
use futures::pin_mut;

use autoconnect_common::{
    protocol::ServerMessage,
    test_support::{hello_db, HELLO, UA},
};
use autoconnect_settings::{AppState, Settings};
use autoconnect_ws_sm::UnidentifiedClient;

use crate::{error::WSErrorKind, handler::webpush_ws, session::MockSession};

#[ctor::ctor]
fn init_test_logging() {
    autopush_common::logging::init_test_logging();
}

fn uclient(app_state: AppState) -> UnidentifiedClient {
    UnidentifiedClient::new(UA.to_owned(), Arc::new(app_state))
}

#[actix_web::test]
async fn handshake_timeout() {
    let settings = Settings {
        open_handshake_timeout: Duration::from_secs_f32(0.15),
        ..Settings::test_settings()
    };
    let client = uclient(AppState::from_settings(settings).unwrap());

    let s = stream! {
        tokio::time::sleep(Duration::from_secs_f32(0.2)).await;
        yield Ok(actix_ws::Message::Text(HELLO.into()));
    };
    pin_mut!(s);
    let err = webpush_ws(client, &mut MockSession::new(), s)
        .await
        .unwrap_err();
    assert!(matches!(err.kind, WSErrorKind::HandshakeTimeout));
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
        .times(1)
        .withf(|msg| matches!(msg, ServerMessage::Hello { .. }))
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
        auto_ping_interval: Duration::from_secs_f32(0.15),
        ..Settings::test_settings()
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
        tokio::time::sleep(Duration::from_secs_f32(0.2)).await;
    };
    pin_mut!(s);
    webpush_ws(client, &mut session, s)
        .await
        .expect("Handler failed");
}

#[actix_web::test]
async fn auto_ping_timeout() {
    let settings = Settings {
        auto_ping_interval: Duration::from_secs_f32(0.15),
        auto_ping_timeout: Duration::from_secs_f32(0.15),
        ..Settings::test_settings()
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
        tokio::time::sleep(Duration::from_secs_f32(0.35)).await;
    };
    pin_mut!(s);
    let err = webpush_ws(client, &mut session, s).await.unwrap_err();
    assert!(matches!(err.kind, WSErrorKind::PongTimeout));
}

#[actix_web::test]
async fn auto_ping_timeout_after_pong() {
    let settings = Settings {
        auto_ping_interval: Duration::from_secs_f32(0.15),
        auto_ping_timeout: Duration::from_secs_f32(0.15),
        ..Settings::test_settings()
    };
    let client = uclient(AppState {
        db: hello_db().into_boxed_arc(),
        ..AppState::from_settings(settings).unwrap()
    });
    let mut session = MockSession::new();
    session.expect_text().times(1).return_once(|_| Ok(()));
    session.expect_ping().times(2).returning(|_| Ok(()));

    let s = stream! {
        yield Ok(actix_ws::Message::Text(HELLO.into()));
        tokio::time::sleep(Duration::from_secs_f32(0.2)).await;
        yield Ok(actix_ws::Message::Pong("".into()));
        tokio::time::sleep(Duration::from_secs_f32(0.35)).await;
    };
    pin_mut!(s);
    let err = webpush_ws(client, &mut session, s).await.unwrap_err();
    assert!(matches!(err.kind, WSErrorKind::PongTimeout));
}
