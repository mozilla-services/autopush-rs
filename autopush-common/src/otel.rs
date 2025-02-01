use actix_web::{http::header::HeaderMap, FromRequest, HttpRequest};
use opentelemetry::{
    global,
    propagation::Extractor,
    trace::{FutureExt, Span, SpanKind, TraceContextExt, Tracer},
    Context, KeyValue,
};
//use opentelemetry_http::HeaderExtractor;
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::TracerProvider};
use opentelemetry_stdout::SpanExporter;

pub fn init_tracer() {
    global::set_text_map_propagator(TraceContextPropagator::new());

    // Setup tracerprovider with stdout exporter
    // that prints the spans to stdout.
    let provider = TracerProvider::builder()
        .with_simple_exporter(SpanExporter::default())
        .build();

    global::set_tracer_provider(provider);
}

/// Extract the context from the incoming request headers
pub fn extract_context_from_request(req: &HttpRequest) -> Context {
    global::get_text_map_propagator(|propagator| {
        propagator.extract(&ActixHeaderExtractor(req.headers()))
    })
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
