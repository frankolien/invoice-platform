pub mod health;
pub mod metrics;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig, WithHttpConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

const SERVICE_NAME: &str = "invoice-platform";

/// Set up tracing. If `otel_endpoint` is provided, spans are also exported
/// via OTLP HTTP to that endpoint (e.g. Jaeger on `http://localhost:4318`).
///
/// Returns an `Option<SdkTracerProvider>` whose lifetime MUST be kept until
/// program shutdown — `SdkTracerProvider`'s `Drop` impl calls `shutdown()`,
/// and clones of it share the same underlying batch processor, so any drop
/// kills the export pipeline. Hold the returned provider in `main` (not in
/// a short-lived scope) and call `.shutdown()` on it explicitly to flush
/// the final batch on graceful exit.
pub fn init_tracing(otel_endpoint: Option<&str>) -> Option<SdkTracerProvider> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn"));

    let registry = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().json().with_target(false));

    let Some(endpoint) = otel_endpoint else {
        registry.init();
        return None;
    };

    // opentelemetry-otlp 0.31 needs an explicit HTTP client — without
    // .with_http_client, build() succeeds but every export errors with
    // "no http client specified".
    let exporter = match SpanExporter::builder()
        .with_http()
        .with_http_client(reqwest::Client::new())
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(endpoint)
        .build()
    {
        Ok(e) => e,
        Err(err) => {
            eprintln!("OTel exporter setup failed; tracing will stay log-only: {err}");
            registry.init();
            return None;
        }
    };

    // Using simple_exporter (synchronous, one HTTP POST per span) instead of
    // batch — the 0.31 batch processor has lifecycle issues that cause it to
    // stop exporting early in this configuration. Trades throughput for
    // correctness; revisit if span volume gets high.
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_attributes([KeyValue::new("service.name", SERVICE_NAME)])
                .build(),
        )
        .build();

    // Build the tracer from a *clone* of the provider, install the layer,
    // then return the original provider intact. We must not let any clone
    // fall out of scope before the program exits.
    let tracer = provider.tracer(SERVICE_NAME);
    registry
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .init();

    Some(provider)
}
