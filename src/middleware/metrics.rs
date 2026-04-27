//! HTTP metrics middleware.
//!
//! Records `http_requests_total` and `http_request_duration_seconds` for
//! every request that flows through. Uses the matched route pattern (e.g.
//! `/v1/invoices/{id}`) rather than the raw URL so cardinality stays bounded.

use std::time::Instant;

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{Error, web};

use crate::observability::metrics::Metrics;

pub async fn http_metrics(
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let metrics = req.app_data::<web::Data<Metrics>>().cloned();
    let start = Instant::now();
    let method = req.method().clone();

    let resp = next.call(req).await?;

    if let Some(m) = metrics {
        // match_pattern() returns the matched scope+route template if any,
        // else falls back to the raw path. Falling back to "unknown" keeps
        // the cardinality fixed even if a 404 hits unrouted paths.
        let route = resp
            .request()
            .match_pattern()
            .unwrap_or_else(|| "unknown".to_string());
        let status = resp.status().as_u16().to_string();
        let labels = &[method.as_str(), route.as_str(), status.as_str()];
        m.http_requests_total.with_label_values(labels).inc();
        m.http_request_duration
            .with_label_values(labels)
            .observe(start.elapsed().as_secs_f64());
    }

    Ok(resp)
}
