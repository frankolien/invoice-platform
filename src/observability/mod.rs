pub mod health;
pub mod metrics;

use opentelemetry::trace::TracerProvider as _;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

const SERVICE_NAME: &str = "invoice-platform";

/// Set up tracing. If `otel_endpoint` is provided, spans are also exported
/// via OTLP HTTP to that endpoint (e.g. Jaeger on `http://localhost:4318`).
/// Returns an optional `TracerProvider` whose lifetime should be kept until
/// shutdown so spans can be flushed.
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

    // OTLP HTTP exporter -> Jaeger (or any OTLP collector).
    // The endpoint should be the receiver root; the SDK appends `/v1/traces`.
    let exporter = match SpanExporter::builder()
        .with_http()
        .with_protocol(Protocol::HttpBinary)
        .with_endpoint(endpoint)
        .build()
    {
        Ok(e) => e,
        Err(err) => {
            eprintln!(
                "OTel exporter setup failed; tracing will stay log-only: {err}"
            );
            registry.init();
            return None;
        }
    };

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_attributes([KeyValue::new("service.name", SERVICE_NAME)])
                .build(),
        )
        .build();

    let tracer = provider.tracer(SERVICE_NAME);
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    registry.with(otel_layer).init();

    Some(provider)
}
