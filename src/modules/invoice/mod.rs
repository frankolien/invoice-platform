use actix_web::{HttpResponse, Responder, get, patch, post, web};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::middleware::tenant::TenantContext;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LineItem {
    pub description: String,
    pub quantity: Decimal,
    pub unit_price: Decimal,
}

#[derive(Debug, Deserialize, Validate)]
pub struct CreateInvoiceInput {
    pub client_id: Uuid,
    #[validate(length(min = 1, max = 64))]
    pub invoice_number: String,
    #[validate(length(min = 1))]
    pub line_items: Vec<LineItem>,
    pub tax_rate: Option<Decimal>,
    pub currency: Option<String>,
    pub due_date: DateTime<Utc>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateInvoiceInput {
    pub line_items: Option<Vec<LineItem>>,
    pub tax_rate: Option<Decimal>,
    pub currency: Option<String>,
    pub due_date: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Invoice {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub invoice_number: String,
    pub status: String,
    pub line_items: JsonValue,
    pub subtotal: Decimal,
    pub tax_rate: Decimal,
    pub tax_amount: Decimal,
    pub total: Decimal,
    pub currency: String,
    pub due_date: DateTime<Utc>,
    pub notes: Option<String>,
    pub sent_at: Option<DateTime<Utc>>,
    pub viewed_at: Option<DateTime<Utc>>,
    pub paid_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/invoices")
            .service(create)
            .service(list)
            .service(get_one)
            .service(update)
            .service(send)
            .service(cancel)
            .service(mark_viewed)
            .configure(crate::modules::payment::configure_invoice_scoped),
    );
}

fn compute_totals(items: &[LineItem], tax_rate: Decimal) -> (Decimal, Decimal, Decimal) {
    let subtotal: Decimal = items
        .iter()
        .map(|i| i.quantity * i.unit_price)
        .sum();
    let tax_amount = subtotal * tax_rate;
    let total = subtotal + tax_amount;
    (subtotal, tax_amount, total)
}

#[post("")]
async fn create(
    pool: web::Data<DbPool>,
    queues: web::Data<crate::jobs::JobQueues>,
    metrics: web::Data<crate::observability::metrics::Metrics>,
    tenant: TenantContext,
    input: web::Json<CreateInvoiceInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let invoice = create_invoice(
        pool.get_ref(),
        tenant.org_id,
        input.client_id,
        Some(input.invoice_number.clone()),
        &input.line_items,
        input.tax_rate.unwrap_or(Decimal::ZERO),
        input.currency.clone().unwrap_or_else(|| "USD".to_string()),
        input.due_date,
        input.notes.clone(),
    )
    .await?;

    metrics
        .invoices_created_total
        .with_label_values(&[&tenant.org_id.to_string()])
        .inc();

    crate::jobs::dispatch_webhooks(
        queues.get_ref(),
        pool.get_ref(),
        tenant.org_id,
        "invoice.created",
        serde_json::json!({
            "invoice_id": invoice.id,
            "invoice_number": invoice.invoice_number,
            "total": invoice.total,
            "client_id": invoice.client_id,
        }),
    )
    .await;

    Ok(HttpResponse::Created().json(invoice))
}

/// Auto-generates an invoice number in the format `INV-{year}-{0001}` per org.
/// Uses a table count so it's deterministic within an org-year bucket. Not
/// concurrency-safe beyond the unique(org_id, invoice_number) constraint — if
/// two callers race, one gets a Conflict and the caller can retry.
async fn generate_invoice_number(pool: &DbPool, org_id: Uuid) -> AppResult<String> {
    let year = chrono::Utc::now().format("%Y").to_string();
    let prefix = format!("INV-{year}-");
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*)::bigint FROM invoices WHERE org_id = $1 AND invoice_number LIKE $2",
    )
    .bind(org_id)
    .bind(format!("{prefix}%"))
    .fetch_one(pool)
    .await?;
    Ok(format!("{prefix}{:04}", count.0 + 1))
}

/// Internal invoice-creation service. The HTTP handler passes a user-supplied
/// invoice_number; the recurring-invoice job passes None for auto-generation.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_invoice(
    pool: &DbPool,
    org_id: Uuid,
    client_id: Uuid,
    invoice_number: Option<String>,
    line_items: &[LineItem],
    tax_rate: Decimal,
    currency: String,
    due_date: DateTime<Utc>,
    notes: Option<String>,
) -> AppResult<Invoice> {
    let client_exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM clients WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL",
    )
    .bind(client_id)
    .bind(org_id)
    .fetch_optional(pool)
    .await?;
    if client_exists.is_none() {
        return Err(AppError::BadRequest("client not found in org".into()));
    }

    let invoice_number = match invoice_number {
        Some(n) => n,
        None => generate_invoice_number(pool, org_id).await?,
    };

    let (subtotal, tax_amount, total) = compute_totals(line_items, tax_rate);
    let line_items_json = serde_json::to_value(line_items)
        .map_err(|e| AppError::internal(format!("line items serialize: {e}")))?;

    let invoice: Invoice = sqlx::query_as(
        r#"
        INSERT INTO invoices (
            org_id, client_id, invoice_number, line_items,
            subtotal, tax_rate, tax_amount, total, currency, due_date, notes
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        RETURNING id, org_id, client_id, invoice_number, status, line_items,
                  subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
                  sent_at, viewed_at, paid_at, created_at, updated_at
        "#,
    )
    .bind(org_id)
    .bind(client_id)
    .bind(&invoice_number)
    .bind(&line_items_json)
    .bind(subtotal)
    .bind(tax_rate)
    .bind(tax_amount)
    .bind(total)
    .bind(&currency)
    .bind(due_date)
    .bind(&notes)
    .fetch_one(pool)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            AppError::Conflict("invoice_number already exists in org".into())
        }
        other => AppError::from(other),
    })?;

    Ok(invoice)
}

#[get("")]
async fn list(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    query: web::Query<ListQuery>,
) -> AppResult<impl Responder> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;
    let status = query.status.clone();

    let items: Vec<Invoice> = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, invoice_number, status, line_items,
               subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
               sent_at, viewed_at, paid_at, created_at, updated_at
        FROM invoices
        WHERE org_id = $1 AND ($2::text IS NULL OR status = $2)
        ORDER BY created_at DESC
        LIMIT $3 OFFSET $4
        "#,
    )
    .bind(tenant.org_id)
    .bind(&status)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(items))
}

#[get("/{id}")]
async fn get_one(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let inv: Option<Invoice> = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, invoice_number, status, line_items,
               subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
               sent_at, viewed_at, paid_at, created_at, updated_at
        FROM invoices WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    inv.map(|i| HttpResponse::Ok().json(i))
        .ok_or(AppError::NotFound)
}

#[patch("/{id}")]
async fn update(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
    input: web::Json<UpdateInvoiceInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    let id = path.into_inner();

    let mut tx = pool.begin().await?;

    let current: Invoice = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, invoice_number, status, line_items,
               subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
               sent_at, viewed_at, paid_at, created_at, updated_at
        FROM invoices WHERE id = $1 AND org_id = $2 FOR UPDATE
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    if current.status != "draft" {
        return Err(AppError::BadRequest(
            "only draft invoices can be edited".into(),
        ));
    }

    let line_items: Vec<LineItem> = match &input.line_items {
        Some(items) => items.clone(),
        None => serde_json::from_value(current.line_items.clone())
            .map_err(|e| AppError::internal(format!("line items parse: {e}")))?,
    };
    let tax_rate = input.tax_rate.unwrap_or(current.tax_rate);
    let (subtotal, tax_amount, total) = compute_totals(&line_items, tax_rate);
    let line_items_json = serde_json::to_value(&line_items)
        .map_err(|e| AppError::internal(format!("serialize: {e}")))?;

    let updated: Invoice = sqlx::query_as(
        r#"
        UPDATE invoices SET
            line_items = $3,
            tax_rate = $4,
            subtotal = $5,
            tax_amount = $6,
            total = $7,
            currency = COALESCE($8, currency),
            due_date = COALESCE($9, due_date),
            notes = COALESCE($10, notes),
            updated_at = now()
        WHERE id = $1 AND org_id = $2
        RETURNING id, org_id, client_id, invoice_number, status, line_items,
                  subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
                  sent_at, viewed_at, paid_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .bind(&line_items_json)
    .bind(tax_rate)
    .bind(subtotal)
    .bind(tax_amount)
    .bind(total)
    .bind(&input.currency)
    .bind(input.due_date)
    .bind(&input.notes)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(HttpResponse::Ok().json(updated))
}

#[post("/{id}/send")]
async fn send(
    pool: web::Data<DbPool>,
    queues: web::Data<crate::jobs::JobQueues>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let inv: Option<Invoice> = sqlx::query_as(
        r#"
        UPDATE invoices SET status = 'sent', sent_at = now(), updated_at = now()
        WHERE id = $1 AND org_id = $2 AND status = 'draft'
        RETURNING id, org_id, client_id, invoice_number, status, line_items,
                  subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
                  sent_at, viewed_at, paid_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    let invoice = inv.ok_or_else(|| {
        AppError::BadRequest("invoice not found or not in draft status".into())
    })?;

    // Fire-and-forget enqueue. If this fails, log but don't fail the request —
    // the status transition already committed. The overdue worker + manual
    // resend can recover. (TODO: outbox pattern for at-least-once guarantees.)
    if let Err(e) = crate::jobs::enqueue_invoice_email(queues.get_ref(), invoice.id).await {
        tracing::error!(error = %e, invoice_id = %invoice.id, "failed to enqueue invoice email");
    }

    crate::jobs::dispatch_webhooks(
        queues.get_ref(),
        pool.get_ref(),
        tenant.org_id,
        "invoice.sent",
        serde_json::json!({ "invoice_id": invoice.id, "invoice_number": invoice.invoice_number, "total": invoice.total }),
    )
    .await;

    Ok(HttpResponse::Ok().json(invoice))
}

#[post("/{id}/cancel")]
async fn cancel(
    pool: web::Data<DbPool>,
    queues: web::Data<crate::jobs::JobQueues>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let inv: Option<Invoice> = sqlx::query_as(
        r#"
        UPDATE invoices SET status = 'cancelled', updated_at = now()
        WHERE id = $1 AND org_id = $2
          AND status IN ('draft', 'sent', 'viewed', 'overdue')
        RETURNING id, org_id, client_id, invoice_number, status, line_items,
                  subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
                  sent_at, viewed_at, paid_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    let invoice = inv.ok_or_else(|| AppError::BadRequest("invoice not cancellable".into()))?;

    crate::jobs::dispatch_webhooks(
        queues.get_ref(),
        pool.get_ref(),
        tenant.org_id,
        "invoice.cancelled",
        serde_json::json!({ "invoice_id": invoice.id, "invoice_number": invoice.invoice_number }),
    )
    .await;

    Ok(HttpResponse::Ok().json(invoice))
}

#[post("/{id}/viewed")]
async fn mark_viewed(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let inv: Option<Invoice> = sqlx::query_as(
        r#"
        UPDATE invoices SET
            status = CASE WHEN status = 'sent' THEN 'viewed' ELSE status END,
            viewed_at = COALESCE(viewed_at, now()),
            updated_at = now()
        WHERE id = $1 AND org_id = $2
        RETURNING id, org_id, client_id, invoice_number, status, line_items,
                  subtotal, tax_rate, tax_amount, total, currency, due_date, notes,
                  sent_at, viewed_at, paid_at, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    inv.map(|i| HttpResponse::Ok().json(i)).ok_or(AppError::NotFound)
}
