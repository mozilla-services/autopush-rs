extern crate slog;
#[macro_use]
extern crate slog_scope;
// #[macro_use]
extern crate serde_derive;

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::Instant;
use std::{env, vec::Vec};

use actix::{Actor, ActorContext, StreamHandler};
use actix_http::StatusCode;
use actix_web::middleware::ErrorHandlers;
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use docopt::Docopt;
use serde::Deserialize;
use uuid::Uuid;

use autoconnect_settings::{options::ServerOptions, Settings};
use autoconnect_web::{client::Client, dockerflow};
use autoconnect_ws::ServerNotification;
use autopush_common::errors::{render_404, ApcError, ApcErrorKind, Result};

mod server;

const USAGE: &str = "
Usage: autopush_rs [options]

Options:
    -h, --help                          Show this message.
    --config-connection=CONFIGFILE      Connection configuration file path.
    --config-shared=CONFIGFILE          Common configuration file path.
";


#[derive(Debug, Deserialize)]
struct Args {
    flag_config_connection: Option<String>,
    flag_config_shared: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct AutoConnect {}

impl Actor for AutoConnect {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<std::result::Result<ws::Message, ws::ProtocolError>> for AutoConnect {
    fn handle(
        &mut self,
        msg: std::result::Result<ws::Message, ws::ProtocolError>,
        ctx: &mut Self::Context,
    ) {
        // TODO: Add timeout to drop if no "hello"
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                // TODO: Add megaphone handling
                ctx.pong(&msg);
            }
            Ok(ws::Message::Text(msg)) => {
                info!("{:?}", msg);
                if let Err(e) = self.process_message(msg) {
                    self.process_error(ctx, e)
                };
            }
            _ => {
                error!("Unsupported socket message: {:?}", msg);
                ctx.stop();
                return;
            }
        }
    }
}

impl AutoConnect {
    /// Parse and process the message calling the appropriate sub function
    fn process_message(&mut self, msg: bytestring::ByteString) -> Result<()> {
        // convert msg to JSON / parse
        let bytes = msg.as_bytes();
        let msg: serde_json::Value = serde_json::from_slice(bytes)?;
        if let Some(message_type) = msg.get("messageType") {
            match &message_type.as_str().unwrap().to_lowercase() {
                // TODO: Finish writing these.
                /*
                "hello" => self.process_hello(msg)?,
                ...
                */
                _ => return Err(ApcErrorKind::GeneralError("PLACEHOLDER".to_owned()).into()),
            }
        }
        // match on `messageType`:
        // hello:
        Ok(())
    }

    /// Process the error, logging it and terminating the connection.
    fn process_error(&mut self, ctx: &mut ws::WebsocketContext<AutoConnect>, e: ApcError) {
        // send error to sentry if appropriate
        error!("Error:: {e:?}");
        ctx.stop();
    }
}

async fn ws_handler(
    req: HttpRequest,
    stream: web::Payload,
) -> std::result::Result<HttpResponse, Error> {
    info!("Starting Websocket Service...");
    let state = req.app_data::<ServerOptions>().unwrap();

    // Create the socket to handle routed notifications
    // actix websocket handlers require this to be an async channel.
    let (tx, rx) = mpsc::sync_channel(state.max_pending_notification_queue);
    let connection = Client::new(tx);
    // TODO: Add broadcasts to new connection.
    // connection.broadcasts(...)

    let resp = ws::start(connection, &req, stream);
    resp
}

#[actix_web::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    let mut filenames = Vec::new();
    if let Some(shared_filename) = args.flag_config_shared {
        filenames.push(shared_filename);
    }
    if let Some(config_filename) = args.flag_config_connection {
        filenames.push(config_filename);
    }
    let settings = Settings::with_env_and_config_files(&filenames)
        .map_err(|e| ApcErrorKind::ConfigError(e))?;

    //TODO: Eventually this will match between the various storage engines that
    // we support. For now, it's just the one, DynamoDB.
    // Perform any app global storage initialization.
    if autopush_common::db::StorageType::from_dsn(&settings.db_dsn)
        == autopush_common::db::StorageType::DYNAMODB
    {
        env::set_var(
            "AWS_LOCAL_DYNAMODB",
            settings.db_dsn.clone().unwrap().to_owned(),
        );
    }

    // Sentry requires the environment variable "SENTRY_DSN".
    if env::var("SENTRY_DSN")
        .unwrap_or_else(|_| "".to_owned())
        .is_empty()
    {
        print!("SENTRY_DSN not set. Logging disabled.");
    }

    let _guard = sentry::init(sentry::ClientOptions {
        release: sentry::release_name!(),
        session_mode: sentry::SessionMode::Request, // new session per request
        auto_session_tracking: true,                // new session per request
        // attach_stacktrace: true, // attach a stack trace to ALL messages (not just exceptions)
        // send_default_pii: false, // do not include PII in message
        ..Default::default()
    });

    let server_opts = ServerOptions::from_settings(&settings)?;

    info!("Starting autoconnect on port {:?}", &settings.port);
    HttpServer::new(move || {
        let client_channels:HashMap<Uuid, mpsc::Receiver<ServerNotification>> = HashMap::new();
        App::new()
            .app_data(server_opts.clone())
            .app_data(Arc::new(client_channels))
            .wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, render_404))
            // use the default sentry wrapper for now.
            // TODO: Look into the sentry_actx hub scope? How do we pass actix service request data in?
            .wrap(sentry_actix::Sentry::new()) // Use the default wrapper
            // Websocket Handler
            .route("/ws/", web::get().to(ws_handler))
            // TODO: Internode Message handler
            //.service(web::resource("/push/{uaid}").route(web::push().to(autoconnect_web::route::InterNode::put))
            .service(web::resource("/status").route(web::get().to(dockerflow::status_route)))
            .service(web::resource("/health").route(web::get().to(dockerflow::health_route)))
            .service(web::resource("/v1/err").route(web::get().to(dockerflow::log_check)))
            // standardized
            .service(web::resource("/__error__").route(web::get().to(dockerflow::log_check)))
            // Dockerflow
            .service(web::resource("/__heartbeat__").route(web::get().to(dockerflow::health_route)))
            .service(
                web::resource("/__lbheartbeat__")
                    .route(web::get().to(dockerflow::lb_heartbeat_route)),
            )
            .service(web::resource("/__version__").route(web::get().to(dockerflow::version_route)))
    })
    .bind(("0.0.0.0", settings.port))?
    .run()
    .await
    .map_err(|e| e.into())
    .and_then(|v| {
        info!("Shutting down autoconnect");
        Ok(v)
    })
}
