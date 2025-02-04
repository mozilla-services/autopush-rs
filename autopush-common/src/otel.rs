use opentelemetry::global;
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
