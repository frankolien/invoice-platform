//! Per-tenant rate limiter — fixed window counter in Redis.
//!
//! Limit keys:
//!   - `rate:org:{org_id}:{bucket_minute}` if the request has `x-org-id`
//!   - `rate:ip:{ip}:{bucket_minute}` otherwise
//!
//! Default 200 req/min (matches the TS app). Configurable via
//! `RATE_LIMIT_PER_MIN` — set high (e.g. 100000) when load testing from
//! a single IP. Setting 0 disables the limit entirely.
//!
//! Why fixed window over token bucket: simpler, no floating-point math, one
//! INCR per request. The edge case (a client can burst 2×limit at window
//! boundaries) doesn't matter at this scale; if it ever does, swap to a
//! sliding-log or leaky-bucket script.

use actix_web::body::{EitherBody, MessageBody};
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::middleware::Next;
use actix_web::{Error, HttpResponse, web};
use serde_json::json;

use crate::cache::Cache;
use crate::config::Config;

pub async fn rate_limit(
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<EitherBody<impl MessageBody>>, Error> {
    let path = req.path();
    if path == "/health" || path == "/ready" || path == "/metrics" {
        return next.call(req).await.map(|r| r.map_into_left_body());
    }

    // 0 = disabled, useful for load tests.
    let limit = req
        .app_data::<web::Data<Config>>()
        .map(|c| c.rate_limit_per_min)
        .unwrap_or(200);
    if limit == 0 {
        return next.call(req).await.map(|r| r.map_into_left_body());
    }

    let Some(cache) = req.app_data::<web::Data<Cache>>().cloned() else {
        // Cache missing — fail open rather than rejecting every request.
        return next.call(req).await.map(|r| r.map_into_left_body());
    };

    let key = bucket_key(&req);
    match cache.incr_with_ttl(&key, 60).await {
        Ok(count) if count > limit => {
            let retry_after = cache.ttl(&key).await.ok().unwrap_or(60).max(1);
            tracing::warn!(key = %key, count, "rate limited");
            let resp = HttpResponse::TooManyRequests()
                .insert_header(("retry-after", retry_after.to_string()))
                .json(json!({
                    "error": {
                        "message": "rate limit exceeded",
                        "status": 429,
                    }
                }));
            Ok(req.into_response(resp).map_into_right_body())
        }
        Ok(_) => next.call(req).await.map(|r| r.map_into_left_body()),
        Err(e) => {
            tracing::error!(error = %e, "rate limiter Redis error; failing open");
            next.call(req).await.map(|r| r.map_into_left_body())
        }
    }
}

fn bucket_key(req: &ServiceRequest) -> String {
    // Minute bucket. Cheap to compute, aligns with the 60s TTL.
    let minute = chrono::Utc::now().timestamp() / 60;

    if let Some(org) = req
        .headers()
        .get("x-org-id")
        .and_then(|v| v.to_str().ok())
    {
        return format!("rate:org:{org}:{minute}");
    }

    let ip = req
        .connection_info()
        .realip_remote_addr()
        .unwrap_or("unknown")
        .to_string();
    format!("rate:ip:{ip}:{minute}")
}
