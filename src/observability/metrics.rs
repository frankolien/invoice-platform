use actix_web::{HttpResponse, Responder, get, web};
use prometheus::{
    Encoder, HistogramVec, IntCounterVec, Registry, TextEncoder,
    histogram_opts, opts, register_histogram_vec_with_registry,
    register_int_counter_vec_with_registry,
};

#[derive(Clone)]
pub struct Metrics {
    pub registry: Registry,
    pub http_requests_total: IntCounterVec,
    pub http_request_duration: HistogramVec,
}

impl Metrics {
    pub fn new() -> anyhow::Result<Self> {
        let registry = Registry::new();

        let http_requests_total = register_int_counter_vec_with_registry!(
            opts!("http_requests_total", "Total HTTP requests"),
            &["method", "path", "status"],
            registry
        )?;

        let http_request_duration = register_histogram_vec_with_registry!(
            histogram_opts!(
                "http_request_duration_seconds",
                "HTTP request duration",
                vec![0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0]
            ),
            &["method", "path", "status"],
            registry
        )?;

        Ok(Self {
            registry,
            http_requests_total,
            http_request_duration,
        })
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(metrics_handler);
}

#[get("/metrics")]
async fn metrics_handler(metrics: web::Data<Metrics>) -> impl Responder {
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
