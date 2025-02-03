use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    http::header::HeaderMap,
    middleware::{from_fn, Next},
    App, Error,
};

use opentelemetry::{
    global,
    propagation::Extractor,
    trace::{Span, SpanKind, Tracer},
};

pub async fn otel_middleware(
    tracer_name: &'static str,
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let parent_cx =
        global::get_text_map_propagator(|prop| prop.extract(&ActixHeaderExtractor(req.headers())));

    let tracer = global::tracer(tracer_name);
    let route: std::borrow::Cow<str> = req
        .match_pattern()
        .map(Into::into)
        .unwrap_or_else(|| "unknown".into());
    let _span = tracer
        .span_builder(format!("{} {route}", req.method()))
        .with_kind(SpanKind::Server)
        .start_with_context(&tracer, &parent_cx);

    // pre-processing
    next.call(req).await
    // post-processing
}

struct ActixHeaderExtractor<'a>(pub &'a HeaderMap);

impl Extractor for ActixHeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(|header| header.as_str()).collect()
    }
}
