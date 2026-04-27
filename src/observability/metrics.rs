use actix_web::{HttpResponse, Responder, get, web};
use prometheus::{
    Encoder, HistogramVec, IntCounterVec, IntGaugeVec, Registry, TextEncoder, histogram_opts,
    opts, register_histogram_vec_with_registry, register_int_counter_vec_with_registry,
    register_int_gauge_vec_with_registry,
};

#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,
    pub http_requests_total: IntCounterVec,
    pub http_request_duration: HistogramVec,
    pub invoices_created_total: IntCounterVec,
    pub payments_processed_total: IntCounterVec,
    pub circuit_breaker_state: IntGaugeVec,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();

        let http_requests_total = register_int_counter_vec_with_registry!(
            opts!("http_requests_total", "Total HTTP requests"),
            &["method", "route", "status"],
            registry
        )?;

        let http_request_duration = register_histogram_vec_with_registry!(
            histogram_opts!(
                "http_request_duration_seconds",
                "HTTP request duration",
                vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
            ),
            &["method", "route", "status"],
            registry
        )?;

        let invoices_created_total = register_int_counter_vec_with_registry!(
            opts!("invoices_created_total", "Invoices created (per org)"),
            &["org_id"],
            registry
        )?;

        let payments_processed_total = register_int_counter_vec_with_registry!(
            opts!(
                "payments_processed_total",
                "Payments terminated (succeeded/failed) per org"
            ),
            &["org_id", "status"],
            registry
        )?;

        // 0 = closed, 1 = open, 2 = half-open. Used by the circuit breakers.
        let circuit_breaker_state = register_int_gauge_vec_with_registry!(
            opts!(
                "circuit_breaker_state",
                "Circuit breaker state: 0=closed, 1=open, 2=half-open"
            ),
            &["service"],
            registry
        )?;

        Ok(Self {
            registry,
            http_requests_total,
            http_request_duration,
            invoices_created_total,
            payments_processed_total,
            circuit_breaker_state,
        })
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(metrics_handler);
}

#[get("/metrics")]
async fn metrics_handler(
    metrics: web::Data<Metrics>,
    breakers: Option<web::Data<crate::circuit_breaker::CircuitBreakers>>,
) -> impl Responder {
    // Snapshot breaker states into the gauge on every scrape — cheap and
    // means Prometheus always sees the current state without us pushing.
    if let Some(b) = breakers.as_deref() {
        b.export_metrics(&metrics);
    }

    let encoder = TextEncoder::new();
    let families = metrics.registry.gather();
    let mut buf = Vec::new();
    if encoder.encode(&families, &mut buf).is_err() {
        return HttpResponse::InternalServerError().finish();
    }
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(buf)
}
