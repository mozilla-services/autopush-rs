use actix_web::{
    rt,
    web::{self, Data},
    App, HttpRequest, HttpResponse, HttpServer,
};
use tokio::sync::mpsc;

pub struct MockSentry {
    pub endpoint_url: String,
    pub request_rx: mpsc::UnboundedReceiver<serde_json::Value>,
}

impl MockSentry {
    pub fn new() -> Self {
        async fn sentry_envelope(
            req: HttpRequest,
            payload: web::Json<serde_json::Value>,
            tx: Data<mpsc::UnboundedSender<serde_json::Value>>,
        ) -> actix_web::error::Result<HttpResponse> {
            trace!(
                "MockSentry: path: {:#?} query_string: {:#?} {:#?} {:#?}",
                req.path(),
                req.query_string(),
                req.connection_info(),
                req.headers()
            );
            // TODO: pass more data for validation
            tx.send(payload.into_inner())
                .map_err(actix_web::error::ErrorServiceUnavailable)?;
            Ok(HttpResponse::Ok()
                .content_type("application/json")
                .body(r#"{"id": "fc6d8c0c43fc4630ad850ee518f1b9d0"}"#.to_owned()))
        }

        let (tx, request_rx) = mpsc::unbounded_channel::<serde_json::Value>();
        let server = HttpServer::new(move || {
            App::new()
                .app_data(Data::new(tx.clone()))
                .route("/", web::get().to(sentry_envelope))
        });
        let server = server
            .bind(("127.0.0.1", 0))
            .expect("Couldn't bind mock_adm");
        let addr = server.addrs().pop().expect("No MockSentry addr");
        rt::spawn(server.run());
        MockSentry {
            endpoint_url: format!("http://{}:{}/", addr.ip(), addr.port()),
            request_rx,
        }
    }

    /// Return the Sentry envelope payload previously recieved
    pub async fn envelope(&mut self) -> Option<serde_json::Value> {
        self.request_rx.try_recv().ok()
    }
}
