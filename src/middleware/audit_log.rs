//! Audit log middleware.
//!
//! Emits a structured log line for every mutating HTTP request
//! (POST / PATCH / PUT / DELETE) with user, org, role, path, method, status.
//!
//! Purely observational — never rejects a request. Reads the JWT from the
//! Authorization header and the org from `x-org-id` if present. We do the
//! parse work again here rather than relying on the extractors because
//! middleware runs before/after extractors and we want the log line to fire
//! even if the handler rejected on, say, a missing `x-org-id`.

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceRequest, ServiceResponse};
use actix_web::http::Method;
use actix_web::middleware::Next;
use actix_web::{Error, web};

use crate::auth::jwt::TokenService;

pub async fn audit_log(
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    let should_audit = matches!(
        *req.method(),
        Method::POST | Method::PATCH | Method::PUT | Method::DELETE
    );

    if !should_audit {
        return next.call(req).await;
    }

    // Snapshot what we'll need post-response. Done *before* calling next
    // because ServiceRequest is consumed by next.call().
    let method = req.method().clone();
    let path = req.path().to_string();
    let org_id = req
        .headers()
        .get("x-org-id")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let user = extract_user(&req);

    let resp = next.call(req).await?;
    let status = resp.status().as_u16();

    tracing::info!(
        audit = true,
        method = %method,
        path = %path,
        status,
        user_id = user.as_deref(),
        org_id = org_id.as_deref(),
        "audit"
    );

    Ok(resp)
}

/// Peek the Bearer token and verify it as an access token, returning
/// `Some(user_id)` if valid. Silent failures are fine — the actual auth
/// check happens inside the handler via the AuthUser extractor.
fn extract_user(req: &ServiceRequest) -> Option<String> {
    let tokens = req.app_data::<web::Data<TokenService>>()?;
    let header = req
        .headers()
        .get(actix_web::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let token = header.strip_prefix("Bearer ")?.trim();
    tokens
        .verify_access(token)
        .ok()
        .map(|c| c.sub.to_string())
}
