//! Internal router HTTP API
//!
//! Accepts PUT requests to deliver notifications to a connected client or trigger
//! a client to check storage.
//!
//! Valid URL's:
//!     PUT /push/UAID      - Deliver notification to a client
//!     PUT /notify/UAID    - Tell a client to check storage

use std::{str, sync::Arc};

use futures::future::Either;

use futures::future::ok;
use futures::{Future, Stream};
use hyper::{self, service::Service, Body, Method, StatusCode};
use uuid::Uuid;

use crate::server::registry::ClientRegistry;

pub struct Push(pub Arc<ClientRegistry>);

impl Service for Push {
    type ReqBody = Body;
    type ResBody = Body;
    type Error = hyper::Error;
    type Future = Box<dyn Future<Item = hyper::Response<Body>, Error = hyper::Error> + Send>;

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
                        Either::A(clients.notify(uaid, msg).then(move |result| {
                            let body = if result.is_ok() {
                                response.status(StatusCode::OK);
                                Body::empty()
                            } else {
                                response.status(StatusCode::NOT_FOUND);
                                Body::from("Client not available.")
                            };
                            Ok(response.body(body).unwrap())
                        }))
                    } else {
                        Either::B(ok(response
                            .status(hyper::StatusCode::BAD_REQUEST)
                            .body("Unable to decode body payload".into())
                            .unwrap()))
                    }
                }));
            }
            (&Method::PUT, "notif", uaid) => {
                return Box::new(clients.check_storage(uaid).then(move |result| {
                    let body = if result.is_ok() {
                        response.status(StatusCode::OK);
                        Body::empty()
                    } else {
                        response.status(StatusCode::NOT_FOUND);
                        Body::from("Client not available.")
                    };
                    Ok(response.body(body).unwrap())
                }));
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
