/// [ed]
/// The original version of Autopush server predates a fair bit of modern
/// architecture.
/// This version uses a custom crafted HTTP futures server that upgrades
/// to websockets. This is because it was created before "actix" was a
/// thing. There's a LOT of code in here that is disposable once a modern
/// framework is integrated.
///
/// For instance, internally there's a complex state machine that currently
/// needs to be maintained due to the heavy `.poll` nature of the code. In
/// a more modern approach the states are far simpler. A connection goes
/// from Unauthorized (a new connection, waiting for the initial `hello`),
/// into a Command loop for the remainder of the connection. The Command
/// loop checks for pending messages, handles any request transactions and
/// waits for the next event (either an outbound message or a inbound
/// command). On connection termination, run a "cleanup" routine to remove
/// the connection info from the router table.
///
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::io;
use std::net::SocketAddr;
use std::panic;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;
use std::thread;
use std::time::{Duration, Instant};

use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use cadence::StatsdClient;
use chrono::Utc;
use fernet::{Fernet, MultiFernet};
use futures::{channel::oneshot, Future, Sink, Stream};
use hyper::{server::conn::Http, StatusCode};
use openssl::ssl::SslAcceptor;
use sentry::{self, capture_message};
use serde_json::{self, json};
use tokio::time::timeout;
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::{Core, Handle, Timeout};
use tokio_tungstenite::{accept_hdr_async, WebSocketStream};
use tungstenite::handshake::server::Request;
use tungstenite::{self, Message};

use autopush_common::db::dynamodb::DynamoStorage;
// use autopush_common::db::postgres::PostgresStorage;
use autopush_common::db::postgres::PgClientImpl;
use autopush_common::db::DbCommandClient;
use autopush_common::errors::{ApiError, ApiErrorKind, ApiResult};
use autopush_common::logging;
use autopush_common::metrics::new_metrics;
use autopush_common::notification::Notification;
//use autopush_common::util::timeout;

use crate::http;
use crate::megaphone::{
    Broadcast, BroadcastChangeTracker, BroadcastSubs, BroadcastSubsInit, MegaphoneAPIResponse,
};
use crate::old_client::Client;
use crate::old_client::UnAuthClientStateFuture;
use crate::server::dispatch::{Dispatch, RequestType};
use crate::server::metrics::metrics_from_opts;
use crate::server::protocol::{BroadcastValue, ClientMessage, ServerMessage};
use crate::server::rc::RcObject;
use crate::server::registry::ClientRegistry;
use crate::server::webpush_io::WebpushIo;
use crate::settings::Settings;

mod aserver;
mod dispatch;
mod megaphone;
mod middleware;
pub mod old_server;
mod old_webpush_socket;
pub mod protocol;
mod rc;
pub mod registry;
mod tls;
mod webpush_io;

const UAHEADER: &str = "User-Agent";

fn ito_dur(seconds: u32) -> Option<Duration> {
    if seconds == 0 {
        None
    } else {
        Some(Duration::new(seconds.into(), 0))
    }
}

fn fto_dur(seconds: f64) -> Option<Duration> {
    if seconds == 0.0 {
        None
    } else {
        Some(Duration::new(
            seconds as u64,
            (seconds.fract() * 1_000_000_000.0) as u32,
        ))
    }
}

pub struct ServerOptions {
    pub router_port: u16,
    pub port: u16,
    pub fernet: MultiFernet,
    pub ssl_key: Option<PathBuf>,
    pub ssl_cert: Option<PathBuf>,
    pub ssl_dh_param: Option<PathBuf>,
    pub open_handshake_timeout: Option<Duration>,
    pub auto_ping_interval: Duration,
    pub auto_ping_timeout: Duration,
    pub max_connections: Option<u32>,
    pub close_handshake_timeout: Option<Duration>,
    pub _message_table_name: String,
    pub _router_table_name: String,
    pub _meta_table_name: Option<String>,
    pub router_url: String,
    pub endpoint_url: String,
    pub statsd_host: Option<String>,
    pub statsd_port: u16,
    pub megaphone_api_url: Option<String>,
    pub megaphone_api_token: Option<String>,
    pub megaphone_poll_interval: Duration,
    pub human_logs: bool,
    pub msg_limit: u32,
    pub db_dsn: Option<String>,
}

impl ServerOptions {
    pub fn from_settings(settings: Settings) -> Result<Self> {
        let crypto_key = &settings.crypto_keys;
        if !(crypto_key.starts_with('[') && crypto_key.ends_with(']')) {
            return Err("Invalid AUTOPUSH_CRYPTO_KEY".into());
        }
        let crypto_key = &crypto_key[1..crypto_key.len() - 1];
        let fernets: Vec<Fernet> = crypto_key
            .split(',')
            .map(|s| s.trim_matches(|c| c == ' ' || c == '"').to_string())
            .map(|key| Fernet::new(&key).expect("Invalid AUTOPUSH_CRYPTO_KEY"))
            .collect();
        let fernet = MultiFernet::new(fernets);

        let router_url = settings.router_url();
        let endpoint_url = settings.endpoint_url();
        Ok(Self {
            port: settings.port,
            fernet,
            router_port: settings.router_port,
            statsd_host: if settings.statsd_host.is_empty() {
                None
            } else {
                Some(settings.statsd_host)
            },
            statsd_port: settings.statsd_port,
            _message_table_name: settings.message_tablename,
            _router_table_name: settings.router_tablename,
            _meta_table_name: settings.meta_tablename,
            router_url,
            endpoint_url,
            ssl_key: settings.router_ssl_key.map(PathBuf::from),
            ssl_cert: settings.router_ssl_cert.map(PathBuf::from),
            ssl_dh_param: settings.router_ssl_dh_param.map(PathBuf::from),
            auto_ping_interval: fto_dur(settings.auto_ping_interval)
                .expect("auto ping interval cannot be 0"),
            auto_ping_timeout: fto_dur(settings.auto_ping_timeout)
                .expect("auto ping timeout cannot be 0"),
            close_handshake_timeout: ito_dur(settings.close_handshake_timeout),
            max_connections: if settings.max_connections == 0 {
                None
            } else {
                Some(settings.max_connections)
            },
            open_handshake_timeout: ito_dur(5),
            megaphone_api_url: settings.megaphone_api_url,
            megaphone_api_token: settings.megaphone_api_token,
            megaphone_poll_interval: ito_dur(settings.megaphone_poll_interval)
                .expect("megaphone poll interval cannot be 0"),
            human_logs: settings.human_logs,
            msg_limit: settings.msg_limit,
            db_dsn: settings.db_dsn,
        })
    }
}

/// Return a static copy of `version.json` from compile time.
pub async fn write_version_file(socket: WebpushIo) -> ApiResult<()> {
    write_json(
        socket,
        StatusCode::OK,
        serde_json::Value::from(include_str!("../../../version.json")),
    )
    .await
}

async fn write_log_check(socket: WebpushIo) -> ApiResult<()> {
    let status = StatusCode::IM_A_TEAPOT;
    let code: u16 = status.into();

    error!("Test Critical Message";
           "status_code" => code,
           "errno" => 0_u16,
    );
    thread::spawn(|| {
        panic!("LogCheck");
    });

    write_json(
        socket,
        StatusCode::IM_A_TEAPOT,
        json!({
                "code": code,
                "errno": 999,
                "error": "Test Failure",
                "mesage": "FAILURE:Success",
        }),
    )
    .await
}

async fn write_json(
    socket: WebpushIo,
    status: StatusCode,
    body: serde_json::Value,
) -> ApiResult<()> {
    let body = body.to_string();
    let data = format!(
        "\
         HTTP/1.1 {status}\r\n\
         Server: webpush\r\n\
         Date: {date}\r\n\
         Content-Length: {len}\r\n\
         Content-Type: application/json\r\n\
         \r\n\
         {body}\
         ",
        status = status,
        date = Utc::now().to_rfc2822(),
        len = body.len(),
        body = body,
    );

    tokio_io::io::write_all(socket, data.into_bytes())
        .map(|_| ())
        .map_err(|_| ApiErrorKind::GeneralError("failed to write status response".to_owned()))
        .await?;
    Ok(())
}
