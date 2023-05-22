use actix_http::ws::{self, Codec};
use actix_test::TestServer;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};

use autoconnect_common::test_support::{hello_again_db, hello_db, DUMMY_UAID, HELLO, HELLO_AGAIN};
use autoconnect_settings::AppState;
use autopush_common::notification::Notification;

use crate::{build_app, config};

fn test_server(app_state: AppState) -> TestServer {
    actix_test::start(move || build_app!(app_state))
}

/// Extract the next message from the pending message queue and attempt to
/// convert it into a parsed JSON Value
async fn json_msg(
    framed: &mut actix_codec::Framed<impl AsyncRead + AsyncWrite + Unpin, Codec>,
) -> serde_json::Value {
    let item = framed.next().await.unwrap().unwrap();
    let ws::Frame::Text(bytes) = item else {
        panic!("Expected Text not: {:#?}", item);
    };
    serde_json::from_slice(&bytes).unwrap()
}

#[actix_rt::test]
pub async fn hello_new_user() {
    let mut srv = test_server(AppState {
        db: hello_db().into_boxed_arc(),
        ..Default::default()
    });

    let mut framed = srv.ws().await.unwrap();
    framed.send(ws::Message::Text(HELLO.into())).await.unwrap();

    let msg = json_msg(&mut framed).await;
    assert_eq!(msg["messageType"], "hello");
    assert_eq!(msg["status"], 200);
    assert_eq!(msg["use_webpush"], true);
    assert!(msg["uaid"].is_string());
    assert!(msg["broadcasts"].is_object());
    assert_eq!(msg.as_object().map_or(0, |o| o.len()), 5);
}

#[actix_rt::test]
pub async fn hello_again() {
    let mut srv = test_server(AppState {
        db: hello_again_db(DUMMY_UAID).into_boxed_arc(),
        ..Default::default()
    });

    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Text(HELLO_AGAIN.into()))
        .await
        .unwrap();

    let msg = json_msg(&mut framed).await;
    assert_eq!(msg["messageType"], "hello");
    assert_eq!(msg["uaid"], DUMMY_UAID.as_simple().to_string());
}

#[actix_rt::test]
pub async fn unsupported_websocket_message() {
    let mut srv = test_server(AppState::default());

    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Binary(HELLO.into()))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    let ws::Frame::Close(Some(close_reason)) = item else {
        panic!("Expected Close(Some(..)) not {:#?}", item);
    };
    assert_eq!(close_reason.code, actix_http::ws::CloseCode::Unsupported);
    assert!(framed.next().await.is_none());
}

#[actix_rt::test]
pub async fn invalid_webpush_message() {
    let mut srv = test_server(AppState {
        db: hello_db().into_boxed_arc(),
        ..Default::default()
    });

    let mut framed = srv.ws().await.unwrap();
    framed.send(ws::Message::Text(HELLO.into())).await.unwrap();

    let msg = json_msg(&mut framed).await;
    assert_eq!(msg["status"], 200);

    framed.send(ws::Message::Text(HELLO.into())).await.unwrap();

    let item = framed.next().await.unwrap().unwrap();
    let ws::Frame::Close(Some(close_reason)) = item else {
        panic!("Expected Close(Some(..)) not {:#?}", item);
    };
    assert_eq!(close_reason.code, actix_http::ws::CloseCode::Error);
    assert!(framed.next().await.is_none());
}

#[actix_rt::test]
pub async fn malformed_webpush_message() {
    let mut srv = test_server(AppState {
        db: hello_db().into_boxed_arc(),
        ..Default::default()
    });

    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Text(
            json!({"messageType": "foo"}).to_string().into(),
        ))
        .await
        .unwrap();

    let item = framed.next().await.unwrap().unwrap();
    let ws::Frame::Close(Some(close_reason)) = item else {
        panic!("Expected Close(Some(..)) not {:#?}", item);
    };
    assert_eq!(close_reason.code, actix_http::ws::CloseCode::Error);
    assert_eq!(close_reason.description.unwrap(), "Json");
    assert!(framed.next().await.is_none());
}

#[actix_rt::test]
pub async fn direct_notif() {
    let app_state = AppState {
        db: hello_again_db(DUMMY_UAID).into_boxed_arc(),
        ..Default::default()
    };
    let mut srv = test_server(app_state.clone());

    let mut framed = srv.ws().await.unwrap();
    framed
        .send(ws::Message::Text(HELLO_AGAIN.into()))
        .await
        .unwrap();

    let msg = json_msg(&mut framed).await;
    assert_eq!(msg["messageType"], "hello");

    app_state
        .clients
        .notify(
            DUMMY_UAID,
            Notification {
                data: Some("foo".to_owned()),
                ..Notification::default()
            },
        )
        .await
        .unwrap();

    // Is a small sleep/tick needed here?
    let msg = json_msg(&mut framed).await;
    assert_eq!(msg["messageType"], "notification");
    assert_eq!(msg["data"], "foo");
}
