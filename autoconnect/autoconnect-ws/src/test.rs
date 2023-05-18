use std::{sync::Arc, time::Duration};

use async_stream::stream;
use futures::pin_mut;
use serde_json::json;

use autoconnect_common::{
    protocol::{BroadcastValue, ServerMessage},
    test_support::{hello_db, HELLO, UA},
};
use autoconnect_settings::{AppState, Settings};
use autoconnect_ws_sm::UnidentifiedClient;

use crate::{error::WSError, handler::webpush_ws, session::MockSession};

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
        ..Default::default()
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
        tokio::time::sleep(Duration::from_secs_f32(0.35)).await;
    };
    pin_mut!(s);
    let err = webpush_ws(client, &mut session, s).await.unwrap_err();
    assert!(matches!(err, WSError::PongTimeout));
}

#[actix_web::test]
async fn auto_ping_timeout_after_pong() {
    let settings = Settings {
        auto_ping_interval: Duration::from_secs_f32(0.15),
        auto_ping_timeout: Duration::from_secs_f32(0.15),
        ..Default::default()
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
    assert!(matches!(err, WSError::PongTimeout));
}

#[actix_web::test]
async fn broadcast() {
    let settings = Settings {
        auto_ping_interval: Duration::from_secs_f32(0.15),
        auto_ping_timeout: Duration::from_secs_f32(0.15),
        ..Default::default()
    };
    let app_state = Arc::new(AppState {
        db: hello_db().into_boxed_arc(),
        ..AppState::from_settings(settings).unwrap()
    });
    app_state
        .broadcaster
        .write()
        .await
        .add_broadcast(("foo/bar".to_owned(), "v1".to_owned()).into());
    let client = UnidentifiedClient::new(UA.to_owned(), Arc::clone(&app_state));

    let mut seq = mockall::Sequence::new();
    let mut session = MockSession::new();
    session
        .expect_text()
        .times(1)
        .in_sequence(&mut seq)
        .withf(|msg| matches!(msg, ServerMessage::Hello { .. }))
        .return_once(|_| Ok(()));
    session
        .expect_ping()
        .times(1)
        .in_sequence(&mut seq)
        .return_once(|_| Ok(()));
    session
        .expect_text()
        .times(1)
        .in_sequence(&mut seq)
        .withf(|smsg| {
            matches!(
                smsg,
                ServerMessage::Broadcast { broadcasts }
                if broadcasts.get("foo/bar") == Some(&BroadcastValue::Value("v2".to_owned())),
            )
        })
        .return_once(|_| Ok(()));

    let hello = json!({"messageType": "hello", "use_webpush": true,
                       "broadcasts": {"foo/bar": "v1"}});
    let s = stream! {
        yield Ok(actix_ws::Message::Text(hello.to_string().into()));
        // Wait for a Ping
        tokio::time::sleep(Duration::from_secs_f32(0.2)).await;
        app_state
            .broadcaster
            .write()
            .await
            .add_broadcast(("foo/bar".to_owned(), "v2".to_owned()).into());
        yield Ok(actix_ws::Message::Pong("".into()));
        // Wait for a Broadcast
        tokio::time::sleep(Duration::from_secs_f32(0.2)).await;
    };
    pin_mut!(s);
    webpush_ws(client, &mut session, s)
        .await
        .expect("Handler failed");
}
