use crate::server::header_util::{get_header, get_owned_header};
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future;

pub struct WebPushHeaders {
    pub authorization: String,
    pub ttl: Option<u64>,
    pub topic: Option<String>,
    pub api_version: String,
}

impl FromRequest for WebPushHeaders {
    type Error = ();
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        let api_version = req
            .match_info()
            .get("api_version")
            .unwrap_or("v1")
            .to_string();

        future::ok(WebPushHeaders {
            authorization: get_owned_header(req, "authorization").unwrap_or_default(),
            ttl: get_header(req, "ttl").and_then(|ttl| ttl.parse().ok()),
            topic: get_owned_header(req, "topic"),
            api_version,
        })
    }
}
