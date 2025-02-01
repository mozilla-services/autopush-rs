use actix_web::{dev::Payload, FromRequest, HttpRequest};
use futures::future;
use opentelemetry::Context;

use autopush_common::otel::extract_context_from_request;

/// Extracts an OpenTelemetry Context
pub struct OtelContext {
    pub cx: Context,
}

impl FromRequest for OtelContext {
    type Error = std::convert::Infallible;
    type Future = future::Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _: &mut Payload) -> Self::Future {
        future::ok(Self {
            cx: extract_context_from_request(req),
        })
    }
}
