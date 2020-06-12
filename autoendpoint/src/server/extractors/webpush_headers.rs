use crate::error::ApiError;
use crate::server::header_util::{get_header, get_owned_header};
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future;
use lazy_static::lazy_static;
use regex::Regex;
use std::cmp::min;
use validator::Validate;
use validator_derive::Validate;

lazy_static! {
    static ref VALID_BASE64_URL: Regex = Regex::new(r"^[0-9A-Za-z\-_]+=*$").unwrap();
}

const MAX_TTL: u64 = 60 * 60 * 24 * 60;

/// Extractor for webpush headers which need validation
#[derive(Validate)]
pub struct WebPushHeaders {
    #[validate(range(min = 0, message = "TTL must be greater than 0", code = "114"))]
    pub ttl: Option<u64>,

    #[validate(
        length(
            max = 32,
            message = "Topic must be no greater than 32 characters",
            code = "113"
        ),
        regex(
            path = "VALID_BASE64_URL",
            message = "Topic must be URL and Filename safe Base64 alphabet",
            code = "113"
        )
    )]
    pub topic: Option<String>,
}

impl FromRequest for WebPushHeaders {
    type Error = ApiError;
    type Future = future::Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        let ttl = get_header(req, "ttl")
            .and_then(|ttl| ttl.parse().ok())
            // Enforce a maximum TTL, but don't error
            .map(|ttl| min(ttl, MAX_TTL));
        let topic = get_owned_header(req, "topic");

        let headers = WebPushHeaders { ttl, topic };
        match headers.validate() {
            Ok(_) => future::ok(headers),
            Err(e) => future::err(ApiError::from(e)),
        }
    }
}
