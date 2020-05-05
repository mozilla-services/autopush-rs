//! Internal router HTTP API
//!
//! Accepts PUT requests to deliver notifications to a connected client or trigger
//! a client to check storage.
//!
//! Valid URL's:
//!     PUT /push/UAID      - Deliver notification to a client
//!     PUT /notify/UAID    - Tell a client to check storage

use std::pin::Pin;
use std::{str, sync::Arc};

use futures::future::ok;
use futures::Future;
use hyper::{self, service::Service, Body, Method, StatusCode};
use serde_json;
use uuid::Uuid;

use crate::server::registry::ClientRegistry;

pub struct Push(pub Arc<ClientRegistry>);

impl Service<http::Request<Body>> for Push {
    type Response = Body;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Body, hyper::Error>>>>;

    fn call(&mut self, req: hyper::Request<Body>) -> Self::Future {
        let mut response = hyper::Response::builder();
        let req_path = req.uri().path().to_string();
        let path_vec: Vec<&str> = req_path.split('/').collect();
        if path_vec.len() != 3 {
            response.status(StatusCode::NOT_FOUND);
            return Box::new(ok(response.body(Body::empty()).unwrap()));
        }
        let (method_name, uaid) = (path_vec[1], path_vec[2]);
        let uaid = match Uuid::parse_str(uaid) {
            Ok(id) => id,
            Err(_) => {
                debug!("uri not uuid: {}", req.uri().to_string());
                response.status(StatusCode::BAD_REQUEST);
                return Box::new(ok(response.body(Body::empty()).unwrap()));
            }
        };
        let clients = Arc::clone(&self.0);
        match (req.method(), method_name, uaid) {
            (&Method::PUT, "push", uaid) => {
                // Due to consumption of body as a future we must return here
                let body = req.into_body().concat2();
                return Box::new(body.and_then(move |body| {
                    let s = String::from_utf8(body.to_vec()).unwrap();
                    if let Ok(msg) = serde_json::from_str(&s) {
                        if clients.notify(uaid, msg).is_ok() {
                            Ok(hyper::Response::builder()
                                .status(StatusCode::OK)
                                .body(Body::empty())
                                .unwrap())
                        } else {
                            Ok(hyper::Response::builder()
                                .status(StatusCode::BAD_GATEWAY)
                                .body(Body::from("Client not available."))
                                .unwrap())
                        }
                    } else {
                        Ok(hyper::Response::builder()
                            .status(hyper::StatusCode::BAD_REQUEST)
                            .body("Unable to decode body payload".into())
                            .unwrap())
                    }
                }));
            }
            (&Method::PUT, "notif", uaid) => {
                if clients.check_storage(uaid).is_ok() {
                    response.status(StatusCode::OK);
                } else {
                    response.status(StatusCode::BAD_GATEWAY);
                    return Box::new(ok(response
                        .body(Body::from("Client not available."))
                        .unwrap()));
                }
            }
            (_, "push", _) | (_, "notif", _) => {
                response.status(StatusCode::METHOD_NOT_ALLOWED);
            }
            _ => {
                response.status(StatusCode::NOT_FOUND);
            }
        };
        Box::new(ok(response.body(Body::empty()).unwrap()))
    }
}
