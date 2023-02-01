extern crate slog;
#[macro_use]
extern crate slog_scope;
#[macro_use]
extern crate serde_derive;

use std::{env, vec::Vec};

use actix::{Actor, ActorContext, StreamHandler};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer};
use actix_web_actors::ws;
use docopt::Docopt;
use serde::Deserialize;

use autoconnect_settings::{options::ServerOptions, Settings};
use autoconnect_web::dockerflow;
use autopush_common::errors::{ApcErrorKind, Result};

mod server;

struct AutoConnect;

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

impl Actor for AutoConnect {
    type Context = ws::WebsocketContext<Self>;
}

impl StreamHandler<std::result::Result<ws::Message, ws::ProtocolError>> for AutoConnect {
    fn handle(&mut self, msg: std::result::Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => {
                // TODO: Add megaphone handling
                ctx.pong(&msg);
            }
            Ok(ws::Message::Text(msg)) => {
                // TODO: self.process_message(msg)
                info!("{:?}", msg);
            }
            _ => {
                error!("Unsupported socket message: {:?}", msg);
                ctx.stop();
                return;
            }
        }
    }
}

async fn ws_handler(req: HttpRequest, stream: web::Payload) -> std::result::Result<HttpResponse, Error> {
    info!("Starting connection...");
    let resp = ws::start(AutoConnect {}, &req, stream);
    info!("Shutting down: {:?}", &resp);
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
    // TODO: move this into the DbClient setup
    if let Some(ddb_local) = settings.db_dsn.clone() {
        if autopush_common::db::StorageType::from_dsn(&ddb_local)
            == autopush_common::db::StorageType::DYNAMODB
        {
            env::set_var("AWS_LOCAL_DYNAMODB", ddb_local.to_owned());
        }
    }

    // Sentry uses the environment variable "SENTRY_DSN".
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

    HttpServer::new(move || {
        App::new()
            .app_data(server_opts.clone())
            //.wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, ApiError::render_404))
            // use the default sentry wrapper for now.
            // TODO: Look into the sentry_actx hub scope? How do we pass actix service request data in?
            .wrap(sentry_actix::Sentry::new()) // Use the default wrapper
            .route("/ws/", web::get().to(ws_handler))
            // Add router info
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
}
