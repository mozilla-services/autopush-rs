//! actix4 based websocket compliant server

use std::sync::Arc;

use actix::{Actor, ActorContext, StreamHandler};
use actix_cors::Cors;
use actix_http::ws::{CloseCode, CloseReason};
use actix_web::{
    dev, http::StatusCode, middleware::ErrorHandlers, web, App, HttpRequest, HttpResponse,
    HttpServer,
};
use actix_web_actors::ws;
use cadence::StatsdClient;
use fernet::MultiFernet;

use crate::aclient::Client;
use crate::http::Push;
use crate::routes::health;
use crate::server::middleware::sentry::SentryWrapper;
use autopush_common::db::client::DbClient;
use autopush_common::db::dynamodb::DdbClientImpl;
use autopush_common::db::postgres::PgClientImpl;
use autopush_common::db::DbCommandClient;
use autopush_common::errors::render_404;
use autopush_common::errors::{ApiError, ApiErrorKind};
use autopush_common::metrics;
use autopush_common::tags::Tags;

use crate::settings::Settings;
// TODO: Port DbClient from autoendpoint to autopush_common?
// TODO: Port SentryWrapper from autoendpoint to autopush_common?
use autopush_common::errors::ApiResult;

/*
/// Generic socket handler WebSocket connections.
pub struct SocketHandler;

impl Actor for SocketHandler {
    type Context = ws::WebsocketContext<Self>;

    /// Called on actor start.
    fn started(&mut self, ctx: &mut Self::Context) {
        // TOOD: if unauthorised, timeout 'til "hello"
        // if authorized, set ping timeout
    }
}

// TODO: finish this
// handle the various websocket message types, passing off to proper functions.
// duplicate with aclient::Client::handle()
impl StreamHandler<ApiResult<ws::Message>> for SocketHandler {
    fn handle(&mut self, msg: ApiResult<ws::Message>, ctx: &mut Self::Context) {
        // process websocket messages
        match msg {
            Ok(ws::Message::Ping(m)) => {
                // TODO: Add in Megaphone's function here?
                // Megaphone supplements the "ping" message with a data packet response.
                // normally, the server sends pings out the the client.
                ctx.pong(&m);
            }
            Ok(ws::Message::Pong(_)) => {
                // TODO: Probably safe to ignore, unless you want to do a Megaphone exchange as well.
            }
            Ok(ws::Message::Text(text)) => return self.do_command(text),
            Ok(ws::Message::Binary(_)) => {
                error!("Unsupported call");
                ctx.close(Some(CloseReason{
                    code: CloseCode::Invalid,
                    description: Some("Unsupported".to_owned())
                }));
                ctx.terminate();
            }
            Ok(ws::Message::Close(reason)) => {
                info!("Closing, reason: {:?}", reason);
                ctx.close(Some(CloseReason{
                    code: CloseCode::Invalid,
                    description: Some("Unsupported".to_owned())
                }));
                ctx.terminate();
            }
            _ =>
            {
                ctx.close(Some(CloseReason{
                    code: CloseCode::Invalid,
                    description: Some("Unsupported".to_owned())
                }));
                ctx.terminate();
            }
        }
    }
}

impl SocketHandler {
    pub async fn do_command(&mut self, cmd: &[u8]) -> ApiResult<()> {
        //
    }
}
// */

/// common server data
pub struct PushServerData;

/// Generic server object.
pub struct Server;

pub struct ServerState {
    pub metrics: Arc<StatsdClient>,
    pub settings: Settings,
    pub fernet: Arc<MultiFernet>,
    pub db_client: Box<dyn DbClient>,
}

impl Server {
    async fn socket_handler(
        &self,
        req: HttpRequest,
        stream: web::Payload,
    ) -> ApiResult<HttpResponse> {
        ws::start(Client::default(), &req, stream)
            .map_err(|e| ApiErrorKind::GeneralError(e.to_string()).into())
    }

    // TODO: Finish this as an Async Server builder.
    async fn with_settings(&self, settings: &Settings) -> ApiResult<dev::Server> {
        let metrics = Arc::new(metrics::new_metrics(
            Some(settings.statsd_host),
            settings.statsd_port,
        )?);
        let bind_address = format!(
            "{}:{}",
            settings.hostname.unwrap_or("localhost".into()),
            settings.port
        );
        let fernet = Arc::new(settings.make_fernet());
        let endpoint_url = settings.endpoint_url();
        let db_settings: autopush_common::db::DbSettings = settings.into();
        let db: Box<dyn DbClient> = match settings.get_storage_type() {
            crate::settings::Storage_Type::DDB => {
                trace!("Using DDB Client");
                Box::new(DdbClientImpl::new(metrics.clone(), settings)?)
            }
            crate::settings::Storage_Type::Postgres => {
                trace!("Using postgres Client");
                Box::new(PgClientImpl::new(metrics.clone(), settings).await?)
            }
            _ => {
                panic!("Unsupported storage type");
            }
        };
        let state = ServerState {
            metrics,
            settings,
            fernet,
            db_client: db,
        };

        // to run call the `result.run().await?`
        Ok(HttpServer::new(move || {
            App::new
                .app_data(state.clone())
                // Middleware
                .wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, render_404))
                .wrap(SentryWrapper::default())
                .wrap(Cors::default())
                //Dockerflow
                .service(web::resource("/push/{uaid}").route(web::put().to(Push::push)))
                .service(web::resource("/notif/{uaid}").route(web::put().to(Push::notif)))
                .service(web::resource("/__heartbeat__").route(web::get().to(health::health_route)))
                .service(
                    web::resource("/__lbheartbeat__")
                        .route(web::get().to(health::lb_heartbeat_route)),
                )
                .service(web::resource("/__version__").route(web::get().to(health::version_route)))
                // websocket handler
                .service(web::resource("/").route(web::get().to(self.socket_handler)))
                .route("/")
        })
        .bind(bind_address))
    }
}
