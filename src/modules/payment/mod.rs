mod idempotency;
pub mod stripe_client;
pub mod webhook;

use std::str::FromStr;

use actix_web::{HttpRequest, HttpResponse, Responder, get, post, web};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use stripe::{
    CheckoutSession, CheckoutSessionMode, CreateCheckoutSession,
    CreateCheckoutSessionLineItems, CreateCheckoutSessionLineItemsPriceData,
    CreateCheckoutSessionLineItemsPriceDataProductData, Currency, Refund,
};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::cache::Cache;
use crate::circuit_breaker::{BreakerError, CircuitBreakers};
use crate::config::Config;
use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::middleware::tenant::TenantContext;
use crate::modules::payment::stripe_client::{StripeClient, require_stripe};

#[derive(Debug, Serialize, FromRow, ToSchema)]
pub struct Payment {
    pub id: Uuid,
    pub org_id: Uuid,
    pub invoice_id: Uuid,
    pub stripe_checkout_session_id: Option<String>,
    pub stripe_payment_intent_id: Option<String>,
    pub amount: Decimal,
    pub currency: String,
    pub status: String,
    pub method: Option<String>,
    pub failure_reason: Option<String>,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefundInput {
    /// If None, refund the full amount.
    pub amount: Option<Decimal>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckoutSessionResponse {
    pub payment_id: Uuid,
    pub checkout_session_id: String,
    pub checkout_url: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RefundResponse {
    pub payment_id: Uuid,
    pub stripe_refund_id: String,
    pub status: String,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    // Payment routes under /v1/payments. The invoice-scoped payment routes
    // (/invoices/:id/pay and /invoices/:id/payments) are registered by the
    // invoice module via `configure_invoice_scoped` below so they live
    // inside the existing `/invoices` scope and Actix routes them correctly.
    cfg.service(web::scope("/payments").service(refund));
    // /webhooks/stripe registered separately (no tenant context)
}

/// Routes that need to live inside the `/invoices` scope. Called from
/// `modules::invoice::configure`.
pub fn configure_invoice_scoped(cfg: &mut web::ServiceConfig) {
    cfg.service(create_checkout)
        .service(list_payments_for_invoice);
}

#[utoipa::path(
    post,
    path = "/v1/invoices/{invoice_id}/pay",
    tag = "payments",
    params(
        ("invoice_id" = Uuid, Path, description = "Invoice id"),
        ("Idempotency-Key" = Option<String>, Header, description = "Idempotency key for safe retries"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 201, description = "Stripe Checkout session created", body = CheckoutSessionResponse),
        (status = 400, description = "Invoice cannot be paid in its current status"),
        (status = 404, description = "Invoice not found"),
        (status = 503, description = "Stripe not configured"),
    )
)]
#[post("/{invoice_id}/pay")]
#[allow(clippy::too_many_arguments)]
pub async fn create_checkout(
    req: HttpRequest,
    pool: web::Data<DbPool>,
    cache: web::Data<Cache>,
    cfg: web::Data<Config>,
    breakers: web::Data<CircuitBreakers>,
    stripe: Option<web::Data<StripeClient>>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let stripe = require_stripe(stripe.as_ref())?;
    let invoice_id = path.into_inner();

    // Idempotency-Key short-circuit
    let idem_key = req
        .headers()
        .get("idempotency-key")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());
    if let Some(ref k) = idem_key {
        if let Some(cached) = idempotency::check(&cache, "pay", k).await? {
            return Ok(HttpResponse::Ok().json(cached));
        }
    }

    // Load invoice
    let invoice: InvoiceForPayment = sqlx::query_as(
        r#"
        SELECT id, org_id, invoice_number, total, currency, status, client_id
        FROM invoices WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(invoice_id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::NotFound)?;

    if !matches!(
        invoice.status.as_str(),
        "sent" | "viewed" | "overdue" | "partially_paid"
    ) {
        return Err(AppError::BadRequest(format!(
            "invoice in status '{}' cannot be paid",
            invoice.status
        )));
    }

    // Load client (for email)
    let client: (String,) = sqlx::query_as("SELECT email FROM clients WHERE id = $1")
        .bind(invoice.client_id)
        .fetch_one(pool.get_ref())
        .await?;

    // Amount in smallest currency unit (cents for USD)
    let amount_cents = (invoice.total * Decimal::from(100))
        .round()
        .try_into()
        .map_err(|_| AppError::internal("invoice total too large for Stripe"))?;
    let currency = Currency::from_str(&invoice.currency.to_lowercase())
        .map_err(|_| AppError::BadRequest(format!("unsupported currency: {}", invoice.currency)))?;

    // Pre-create a Payment row in 'pending' so we can correlate webhooks even
    // if the client never completes the session.
    let payment: Payment = sqlx::query_as(
        r#"
        INSERT INTO payments (org_id, invoice_id, amount, currency, status)
        VALUES ($1, $2, $3, $4, 'pending')
        RETURNING id, org_id, invoice_id, stripe_checkout_session_id, stripe_payment_intent_id,
                  amount, currency, status, method, failure_reason, paid_at, created_at, updated_at
        "#,
    )
    .bind(tenant.org_id)
    .bind(invoice_id)
    .bind(invoice.total)
    .bind(&invoice.currency)
    .fetch_one(pool.get_ref())
    .await?;

    // Build Stripe checkout session
    let success_url = format!("{}/invoices/{}/paid", cfg.app_url, invoice_id);
    let cancel_url = format!("{}/invoices/{}", cfg.app_url, invoice_id);

    let payment_id_str = payment.id.to_string();
    let mut params = CreateCheckoutSession::new();
    params.mode = Some(CheckoutSessionMode::Payment);
    params.success_url = Some(&success_url);
    params.cancel_url = Some(&cancel_url);
    params.customer_email = Some(&client.0);
    params.client_reference_id = Some(&payment_id_str);
    params.line_items = Some(vec![CreateCheckoutSessionLineItems {
        price_data: Some(CreateCheckoutSessionLineItemsPriceData {
            currency,
            product_data: Some(CreateCheckoutSessionLineItemsPriceDataProductData {
                name: format!("Invoice {}", invoice.invoice_number),
                ..Default::default()
            }),
            unit_amount: Some(amount_cents),
            ..Default::default()
        }),
        quantity: Some(1),
        ..Default::default()
    }]);
    // Pass our ids through so the webhook can link back without a round-trip.
    let mut meta = std::collections::HashMap::new();
    meta.insert("invoice_id".to_string(), invoice_id.to_string());
    meta.insert("org_id".to_string(), tenant.org_id.to_string());
    meta.insert("payment_id".to_string(), payment.id.to_string());
    params.metadata = Some(meta);

    let session = breakers
        .stripe
        .call(|| CheckoutSession::create(&stripe.client, params))
        .await
        .map_err(|e| match e {
            BreakerError::Open => AppError::Internal(
                "stripe circuit breaker open — try again shortly".into(),
            ),
            BreakerError::Inner(err) => {
                AppError::internal(format!("stripe create session: {err}"))
            }
        })?;

    // Record the session id on our payment row
    sqlx::query(
        "UPDATE payments SET stripe_checkout_session_id = $1, updated_at = now() WHERE id = $2",
    )
    .bind(session.id.as_str())
    .bind(payment.id)
    .execute(pool.get_ref())
    .await?;

    let response = json!({
        "payment_id": payment.id,
        "checkout_session_id": session.id.as_str(),
        "checkout_url": session.url,
    });
    if let Some(ref k) = idem_key {
        idempotency::store_result(&cache, "pay", k, &response).await?;
    }
    Ok(HttpResponse::Created().json(response))
}

#[utoipa::path(
    get,
    path = "/v1/invoices/{invoice_id}/payments",
    tag = "payments",
    params(("invoice_id" = Uuid, Path, description = "Invoice id")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Payments for invoice", body = [Payment]),
        (status = 404, description = "Invoice not found"),
    )
)]
#[get("/{invoice_id}/payments")]
pub async fn list_payments_for_invoice(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let invoice_id = path.into_inner();

    // Confirm invoice belongs to this org (cheap + avoids leaking "exists")
    let invoice_exists: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM invoices WHERE id = $1 AND org_id = $2")
            .bind(invoice_id)
            .bind(tenant.org_id)
            .fetch_optional(pool.get_ref())
            .await?;
    if invoice_exists.is_none() {
        return Err(AppError::NotFound);
    }

    let payments: Vec<Payment> = sqlx::query_as(
        r#"
        SELECT id, org_id, invoice_id, stripe_checkout_session_id, stripe_payment_intent_id,
               amount, currency, status, method, failure_reason, paid_at, created_at, updated_at
        FROM payments WHERE org_id = $1 AND invoice_id = $2
        ORDER BY created_at DESC
        "#,
    )
    .bind(tenant.org_id)
    .bind(invoice_id)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(payments))
}

#[utoipa::path(
    post,
    path = "/v1/payments/{id}/refund",
    tag = "payments",
    params(("id" = Uuid, Path, description = "Payment id")),
    request_body = RefundInput,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Refund issued", body = RefundResponse),
        (status = 400, description = "Payment not refundable"),
        (status = 404, description = "Payment not found"),
        (status = 503, description = "Stripe not configured"),
    )
)]
#[post("/{id}/refund")]
pub async fn refund(
    pool: web::Data<DbPool>,
    breakers: web::Data<CircuitBreakers>,
    stripe: Option<web::Data<StripeClient>>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
    input: web::Json<RefundInput>,
) -> AppResult<impl Responder> {
    let stripe = require_stripe(stripe.as_ref())?;
    let payment_id = path.into_inner();

    let payment: Payment = sqlx::query_as(
        r#"
        SELECT id, org_id, invoice_id, stripe_checkout_session_id, stripe_payment_intent_id,
               amount, currency, status, method, failure_reason, paid_at, created_at, updated_at
        FROM payments WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(payment_id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?
    .ok_or(AppError::NotFound)?;

    if payment.status != "succeeded" {
        return Err(AppError::BadRequest(format!(
            "can only refund succeeded payments (current: {})",
            payment.status
        )));
    }

    let pi_id = payment
        .stripe_payment_intent_id
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("payment has no Stripe payment intent".into()))?;

    let amount_cents: Option<i64> = match input.amount {
        Some(a) if a > Decimal::ZERO => Some(
            (a * Decimal::from(100))
                .round()
                .try_into()
                .map_err(|_| AppError::internal("refund amount too large"))?,
        ),
        _ => None, // full refund
    };

    let mut params = stripe::CreateRefund::new();
    params.payment_intent = Some(
        stripe::PaymentIntentId::from_str(pi_id)
            .map_err(|_| AppError::internal("bad stored payment intent id"))?,
    );
    params.amount = amount_cents;

    let stripe_refund = breakers
        .stripe
        .call(|| Refund::create(&stripe.client, params))
        .await
        .map_err(|e| match e {
            BreakerError::Open => AppError::Internal(
                "stripe circuit breaker open — try again shortly".into(),
            ),
            BreakerError::Inner(err) => AppError::internal(format!("stripe refund: {err}")),
        })?;

    // Update local status: full refund → 'refunded', partial → 'partially_refunded'
    let new_status = if amount_cents.is_some() {
        "partially_refunded"
    } else {
        "refunded"
    };
    sqlx::query("UPDATE payments SET status = $1, updated_at = now() WHERE id = $2")
        .bind(new_status)
        .bind(payment_id)
        .execute(pool.get_ref())
        .await?;

    Ok(HttpResponse::Ok().json(json!({
        "payment_id": payment_id,
        "stripe_refund_id": stripe_refund.id.as_str(),
        "status": new_status,
    })))
}

#[derive(FromRow)]
struct InvoiceForPayment {
    #[allow(dead_code)]
    id: Uuid,
    #[allow(dead_code)]
    org_id: Uuid,
    invoice_number: String,
    total: Decimal,
    currency: String,
    status: String,
    client_id: Uuid,
}
