use actix_web::{
    body::MessageBody,
    dev::{ServiceRequest, ServiceResponse},
    middleware::{from_fn, Next},
    App, Error,
};

use opentelemetry::{
    global,
    trace::{Span, SpanKind, Tracer},
};

pub async fn otel_middleware(
    tracer_name: &str,
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let tracer = global::tracer(tracer_name);
    // XXX: /wpush/{api_version}?
    let mut span = tracer
        .span_builder("wpush")
        .with_kind(SpanKind::Server)
        .start_with_context(&tracer, &cx.cx);
    span.add_event("POST notification", vec![]);

    // pre-processing
    next.call(req).await
    // post-processing
}
