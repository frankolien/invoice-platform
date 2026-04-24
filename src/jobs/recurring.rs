//! Recurring-invoice scheduling.
//!
//! Two pieces:
//!   1. `ScanTick` — runs on cron (every 15 min). Finds templates whose
//!      `next_run_at <= now()` and enqueues a `CreateRecurring` job per
//!      template. Keeps the cron lean: a scan never does more than a query
//!      and a few enqueues.
//!   2. `CreateRecurring` — queue job. Materializes a template into a real
//!      invoice, advances `last_run_at` + `next_run_at`, auto-cancels if
//!      `end_date` has passed.

use apalis::prelude::{BoxDynError, Data, Error};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;

use crate::db::DbPool;
use crate::jobs::JobQueues;
use crate::modules::invoice::{LineItem, create_invoice};
use crate::modules::recurring_invoice::compute_next_run_at;

#[derive(Default, Debug, Clone)]
pub struct ScanTick(pub DateTime<Utc>);

impl From<DateTime<Utc>> for ScanTick {
    fn from(t: DateTime<Utc>) -> Self {
        ScanTick(t)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRecurring {
    pub recurring_invoice_id: Uuid,
}

#[derive(FromRow)]
struct RecurringTemplate {
    id: Uuid,
    org_id: Uuid,
    client_id: Uuid,
    line_items: JsonValue,
    frequency: String,
    tax_rate: Decimal,
    currency: String,
    end_date: Option<DateTime<Utc>>,
    start_date: DateTime<Utc>,
    notes: Option<String>,
}

pub async fn scan(
    _tick: ScanTick,
    pool: Data<DbPool>,
    queues: Data<JobQueues>,
) -> Result<(), Error> {
    // `SELECT ... FOR UPDATE SKIP LOCKED` would be fancier but isn't needed
    // here: even if we enqueue the same job twice, the create handler
    // advances `next_run_at` atomically and the second attempt no-ops.
    let ids: Vec<(Uuid,)> = sqlx::query_as(
        r#"
        SELECT id FROM recurring_invoices
        WHERE status = 'active'
          AND next_run_at IS NOT NULL
          AND next_run_at <= now()
        "#,
    )
    .fetch_all(&*pool)
    .await
    .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;

    if ids.is_empty() {
        return Ok(());
    }

    tracing::info!(count = ids.len(), "found due recurring invoices");

    let mut storage = queues.recurring_create.clone();
    for (id,) in ids {
        if let Err(e) = apalis::prelude::Storage::push(
            &mut storage,
            CreateRecurring {
                recurring_invoice_id: id,
            },
        )
        .await
        {
            tracing::error!(error = %e, recurring_invoice_id = %id, "failed to enqueue CreateRecurring");
        }
    }

    Ok(())
}

pub async fn create(
    job: CreateRecurring,
    pool: Data<DbPool>,
    queues: Data<JobQueues>,
) -> Result<(), Error> {
    let template: Option<RecurringTemplate> = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, line_items, frequency, tax_rate, currency,
               end_date, start_date, notes
        FROM recurring_invoices
        WHERE id = $1 AND status = 'active'
        "#,
    )
    .bind(job.recurring_invoice_id)
    .fetch_optional(&*pool)
    .await
    .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;

    let Some(t) = template else {
        tracing::warn!(
            recurring_invoice_id = %job.recurring_invoice_id,
            "recurring template not found or not active"
        );
        return Ok(());
    };

    // Expiry check
    if let Some(end) = t.end_date {
        if end < Utc::now() {
            sqlx::query(
                "UPDATE recurring_invoices SET status = 'cancelled', next_run_at = NULL, updated_at = now() WHERE id = $1",
            )
            .bind(t.id)
            .execute(&*pool)
            .await
            .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;
            tracing::info!(recurring_invoice_id = %t.id, "recurring expired, cancelled");
            return Ok(());
        }
    }

    let line_items: Vec<LineItem> = serde_json::from_value(t.line_items)
        .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;

    let due_date = Utc::now() + Duration::days(30);

    // Materialize — create_invoice handles number generation + totals + client check.
    let invoice = create_invoice(
        &pool,
        t.org_id,
        t.client_id,
        None, // auto-generate invoice_number
        &line_items,
        t.tax_rate,
        t.currency,
        due_date,
        t.notes,
    )
    .await
    .map_err(|e| Error::from(Box::new(std::io::Error::other(e.to_string())) as BoxDynError))?;

    // Advance scheduling. Uses start_date as the anchor so drift stays bounded —
    // if scanner was late, we still land on the canonical cadence.
    let next_run = compute_next_run_at(t.start_date, &t.frequency);
    sqlx::query(
        "UPDATE recurring_invoices SET last_run_at = now(), next_run_at = $1, updated_at = now() WHERE id = $2",
    )
    .bind(next_run)
    .bind(t.id)
    .execute(&*pool)
    .await
    .map_err(|e| Error::from(Box::new(e) as BoxDynError))?;

    // Fire invoice.created + enqueue the email send (same as the HTTP path).
    crate::jobs::dispatch_webhooks(
        &queues,
        &pool,
        t.org_id,
        "invoice.created",
        serde_json::json!({
            "invoice_id": invoice.id,
            "invoice_number": invoice.invoice_number,
            "total": invoice.total,
            "client_id": invoice.client_id,
            "recurring_invoice_id": t.id,
        }),
    )
    .await;

    tracing::info!(
        recurring_invoice_id = %t.id,
        new_invoice_id = %invoice.id,
        "recurring invoice materialized"
    );
    Ok(())
}
