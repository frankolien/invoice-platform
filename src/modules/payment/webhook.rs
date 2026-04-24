use actix_web::{HttpRequest, HttpResponse, Responder, post, web};
use rust_decimal::Decimal;
use serde_json::json;
use stripe::{EventObject, EventType, Webhook};
use uuid::Uuid;

use crate::cache::Cache;
use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::modules::payment::stripe_client::{StripeClient, require_stripe};

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(web::scope("/webhooks").service(stripe_webhook));
}

#[post("/stripe")]
async fn stripe_webhook(
    req: HttpRequest,
    body: web::Bytes,
    pool: web::Data<DbPool>,
    cache: web::Data<Cache>,
    stripe: Option<web::Data<StripeClient>>,
) -> AppResult<impl Responder> {
    let stripe = require_stripe(stripe.as_ref())?;

    let sig = req
        .headers()
        .get("stripe-signature")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("missing stripe-signature header".into()))?;

    let payload = std::str::from_utf8(&body)
        .map_err(|_| AppError::BadRequest("webhook body is not utf-8".into()))?;

    let event = Webhook::construct_event(payload, sig, &stripe.webhook_secret).map_err(|e| {
        tracing::warn!(error = %e, "stripe webhook signature rejected");
        AppError::Unauthorized
    })?;

    // Dedup via Redis. Stripe retries on non-2xx, so idempotent handling matters.
    let dedup_key = format!("stripe:event:{}", event.id);
    let first_time = cache.set_nx_ex(&dedup_key, "seen", 48 * 60 * 60).await?;
    if !first_time {
        tracing::info!(event_id = %event.id, "stripe event deduplicated");
        return Ok(HttpResponse::Ok().json(json!({ "deduplicated": true })));
    }

    match event.type_ {
        EventType::CheckoutSessionCompleted => {
            if let EventObject::CheckoutSession(session) = event.data.object {
                handle_checkout_completed(&pool, session).await?;
            }
        }
        EventType::PaymentIntentSucceeded => {
            if let EventObject::PaymentIntent(pi) = event.data.object {
                handle_payment_succeeded(&pool, pi).await?;
            }
        }
        EventType::PaymentIntentPaymentFailed => {
            if let EventObject::PaymentIntent(pi) = event.data.object {
                handle_payment_failed(&pool, pi).await?;
            }
        }
        other => {
            tracing::debug!(event_type = ?other, "ignoring stripe event");
        }
    }

    Ok(HttpResponse::Ok().json(json!({ "received": true })))
}

async fn handle_checkout_completed(
    pool: &DbPool,
    session: stripe::CheckoutSession,
) -> AppResult<()> {
    let session_id = session.id.as_str();
    let pi_id = session
        .payment_intent
        .as_ref()
        .map(|p| p.id().to_string())
        .unwrap_or_default();

    // Link the payment intent to our existing payment row (created at /pay time).
    sqlx::query(
        r#"
        UPDATE payments
        SET stripe_payment_intent_id = COALESCE(NULLIF($1, ''), stripe_payment_intent_id),
            updated_at = now()
        WHERE stripe_checkout_session_id = $2
        "#,
    )
    .bind(&pi_id)
    .bind(session_id)
    .execute(pool)
    .await?;

    // Stripe sends payment_intent.succeeded separately which does the status
    // transition; checkout.session.completed just correlates the IDs.
    Ok(())
}

async fn handle_payment_succeeded(
    pool: &DbPool,
    pi: stripe::PaymentIntent,
) -> AppResult<()> {
    let pi_id = pi.id.as_str();

    // Move the payment to succeeded, capture the paid amount/time.
    let updated: Option<(Uuid, Uuid, Decimal)> = sqlx::query_as(
        r#"
        UPDATE payments
        SET status = 'succeeded',
            paid_at = COALESCE(paid_at, now()),
            updated_at = now()
        WHERE stripe_payment_intent_id = $1
        RETURNING id, invoice_id, amount
        "#,
    )
    .bind(pi_id)
    .fetch_optional(pool)
    .await?;

    let Some((_payment_id, invoice_id, amount)) = updated else {
        tracing::warn!(payment_intent_id = %pi_id, "payment_intent.succeeded for unknown payment");
        return Ok(());
    };

    // Recompute invoice status from the sum of succeeded payments.
    transition_invoice_for_payment(pool, invoice_id, amount).await?;
    Ok(())
}

async fn handle_payment_failed(
    pool: &DbPool,
    pi: stripe::PaymentIntent,
) -> AppResult<()> {
    let pi_id = pi.id.as_str();
    let reason = pi
        .last_payment_error
        .as_ref()
        .and_then(|e| e.message.clone())
        .unwrap_or_else(|| "unknown".to_string());

    sqlx::query(
        r#"
        UPDATE payments
        SET status = 'failed',
            failure_reason = $1,
            updated_at = now()
        WHERE stripe_payment_intent_id = $2
        "#,
    )
    .bind(&reason)
    .bind(pi_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Sets the invoice to 'paid' if the sum of succeeded payments covers its total,
/// otherwise 'partially_paid'. Idempotent: safe to call on retries.
async fn transition_invoice_for_payment(
    pool: &DbPool,
    invoice_id: Uuid,
    _new_payment_amount: Decimal,
) -> AppResult<()> {
    let row: (Decimal, Decimal) = sqlx::query_as(
        r#"
        SELECT
            (SELECT total FROM invoices WHERE id = $1) AS total,
            COALESCE(
                (SELECT SUM(amount) FROM payments
                 WHERE invoice_id = $1 AND status = 'succeeded'),
                0
            )::numeric AS paid
        "#,
    )
    .bind(invoice_id)
    .fetch_one(pool)
    .await?;

    let (total, paid) = row;
    let new_status = if paid >= total {
        "paid"
    } else if paid > Decimal::ZERO {
        "partially_paid"
    } else {
        return Ok(());
    };

    sqlx::query(
        r#"
        UPDATE invoices
        SET status = $1,
            paid_at = CASE WHEN $1 = 'paid' THEN COALESCE(paid_at, now()) ELSE paid_at END,
            updated_at = now()
        WHERE id = $2
        "#,
    )
    .bind(new_status)
    .bind(invoice_id)
    .execute(pool)
    .await?;

    Ok(())
}
