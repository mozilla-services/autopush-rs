//! Internal router HTTP API
//!
//! Accepts PUT requests to deliver notifications to a connected client or trigger
//! a client to check storage.
//!
//! Valid URL's:
//!     PUT /push/UAID      - Deliver notification to a client
//!     PUT /notify/UAID    - Tell a client to check storage

use std::rc::Rc;
use std::str;

use futures::future::ok;
use futures::{Future, Stream};
use hyper;
use hyper::{Method, StatusCode};
use serde_json;
use tokio_service::Service;
use uuid::Uuid;

use server::Server;

pub struct Push(pub Rc<Server>);

impl Service for Push {
    type Request = hyper::Request;
    type Response = hyper::Response;
    type Error = hyper::Error;
    type Future = Box<Future<Item = hyper::Response, Error = hyper::Error>>;

    fn call(&self, req: hyper::Request) -> Self::Future {
        let mut response = hyper::Response::new();
        let req_path = req.path().to_string();
        let path_vec: Vec<&str> = req_path.split('/').collect();
        if path_vec.len() != 3 {
            response.set_status(StatusCode::NotFound);
            return Box::new(ok(response));
        }
        let (method_name, uaid) = (path_vec[1], path_vec[2]);
        let uaid = match Uuid::parse_str(uaid) {
            Ok(id) => id,
            Err(_) => {
                debug!("uri not uuid: {}", req.uri().to_string());
                response.set_status(StatusCode::BadRequest);
                return Box::new(ok(response));
            }
        };
        let srv = self.0.clone();
        match (req.method(), method_name, uaid) {
            (&Method::Put, "push", uaid) => {
                // Due to consumption of body as a future we must return here
                let body = req.body().concat2();
                return Box::new(body.and_then(move |body| {
                    let s = String::from_utf8(body.to_vec()).unwrap();
                    if let Ok(msg) = serde_json::from_str(&s) {
                        if srv.notify_client(uaid, msg).is_ok() {
                            Ok(hyper::Response::new().with_status(StatusCode::Ok))
                        } else {
                            Ok(hyper::Response::new()
                                .with_status(StatusCode::BadGateway)
                                .with_body("Client not available."))
                        }
                    } else {
                        Ok(hyper::Response::new()
                            .with_status(hyper::StatusCode::BadRequest)
                            .with_body("Unable to decode body payload"))
                    }
                }));
            }
            (&Method::Put, "notif", uaid) => {
                if srv.check_client_storage(uaid).is_ok() {
                    response.set_status(StatusCode::Ok)
                } else {
                    response.set_status(StatusCode::BadGateway);
                    response.set_body("Client not available.");
                }
            },
            (_, "push", _) | (_, "notif", _) => response.set_status(StatusCode::MethodNotAllowed),
            _ => response.set_status(StatusCode::NotFound),
        };
        Box::new(ok(response))
    }
}
