//! Main application server

use actix_cors::Cors;
use actix_web::{
    dev, http::StatusCode, middleware::errhandlers::ErrorHandlers, web, App, HttpRequest,
    HttpResponse, HttpServer,
};
use cadence::StatsdClient;

use crate::error::ApiError;
use crate::metrics;
use crate::settings::Settings;

#[derive(Clone, Debug)]
pub struct ServerState {
    /// Server Data
    pub metrics: Box<StatsdClient>,
    pub port: u16,
}

pub struct Server;

impl Server {
    pub fn with_settings(settings: Settings) -> Result<dev::Server, ApiError> {
        let metrics = metrics::metrics_from_opts(&settings)?;
        let port = settings.port;
        let state = ServerState {
            metrics: Box::new(metrics),
            port,
        };

        let server = HttpServer::new(move || {
            App::new()
                .data(state.clone())
                .wrap(ErrorHandlers::new().handler(StatusCode::NOT_FOUND, ApiError::render_404))
                .wrap(Cors::default())
                // TODO: Add endpoints and handlers here.
                //
                // Dockerflow
                //.service(web::resource("/__heartbeat__").route(web::get().to(handlers::heartbeat)))
                .service(web::resource("/__lbheartbeat__").route(web::get().to(
                    |_: HttpRequest| {
                        // used by the load balancers, just return OK.
                        HttpResponse::Ok()
                            .content_type("application/json")
                            .body("{}")
                    },
                )))
                .service(
                    web::resource("/__version__").route(web::get().to(|_: HttpRequest| {
                        // return the contents of the version.json file created by circleci
                        // and stored in the docker root
                        HttpResponse::Ok()
                            .content_type("application/json")
                            .body(include_str!("../../version.json"))
                    })),
                )
            //.service(web::resource("/__error__").route(web::get().to(handlers::test_error)))
        })
        .bind(format!("{}:{}", settings.host, settings.port))?
        .run();

        Ok(server)
    }
}
