use actix_web::{HttpResponse, Responder, get, patch, post, web};
use chrono::{DateTime, Datelike, Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::middleware::tenant::TenantContext;
use crate::modules::invoice::LineItem;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateInput {
    pub client_id: Uuid,
    #[validate(length(min = 1))]
    pub line_items: Vec<LineItem>,
    pub frequency: String,
    pub tax_rate: Option<Decimal>,
    pub currency: Option<String>,
    pub start_date: DateTime<Utc>,
    pub end_date: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateInput {
    pub line_items: Option<Vec<LineItem>>,
    pub frequency: Option<String>,
    pub tax_rate: Option<Decimal>,
    pub currency: Option<String>,
    pub end_date: Option<DateTime<Utc>>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub status: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct RecurringInvoice {
    pub id: Uuid,
    pub org_id: Uuid,
    pub client_id: Uuid,
    pub line_items: JsonValue,
    pub frequency: String,
    pub tax_rate: Decimal,
    pub currency: String,
    pub status: String,
    pub start_date: DateTime<Utc>,
    pub end_date: Option<DateTime<Utc>>,
    pub next_run_at: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/recurring-invoices")
            .service(create)
            .service(list)
            .service(get_one)
            .service(update)
            .service(pause)
            .service(resume)
            .service(cancel),
    );
}

fn validate_frequency(f: &str) -> AppResult<()> {
    if !matches!(f, "weekly" | "monthly" | "quarterly" | "yearly") {
        return Err(AppError::BadRequest(format!("invalid frequency: {f}")));
    }
    Ok(())
}

/// Advances `from` repeatedly by `frequency` until the result is strictly in
/// the future. Used when creating/resuming so the very first run never lands
/// in the past and fires an immediate stampede of "due" invoices.
pub fn compute_next_run_at(from: DateTime<Utc>, frequency: &str) -> DateTime<Utc> {
    let mut next = from;
    let now = Utc::now();
    while next <= now {
        next = advance(next, frequency);
    }
    next
}

fn advance(dt: DateTime<Utc>, frequency: &str) -> DateTime<Utc> {
    match frequency {
        "weekly" => dt + Duration::days(7),
        "monthly" => add_months(dt, 1),
        "quarterly" => add_months(dt, 3),
        "yearly" => add_months(dt, 12),
        _ => dt + Duration::days(30),
    }
}

fn add_months(dt: DateTime<Utc>, months: i32) -> DateTime<Utc> {
    // chrono doesn't do "add N months" natively because month length varies.
    // Month-end clamp: March 31 + 1 month -> April 30, not May 1.
    let mut month = dt.month() as i32 + months;
    let mut year = dt.year();
    while month > 12 {
        month -= 12;
        year += 1;
    }
    while month < 1 {
        month += 12;
        year -= 1;
    }
    let day = dt.day().min(last_day_of_month(year, month as u32));
    dt.with_year(year)
        .and_then(|d| d.with_month(month as u32))
        .and_then(|d| d.with_day(day))
        .unwrap_or(dt)
}

fn last_day_of_month(year: i32, month: u32) -> u32 {
    // Trick: first day of next month - 1.
    let (ny, nm) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    chrono::NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|d| d.pred_opt())
        .map(|d| d.day())
        .unwrap_or(28)
}

#[post("")]
async fn create(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    input: web::Json<CreateInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    validate_frequency(&input.frequency)?;

    // Verify client belongs to the org
    let client_exists: Option<(Uuid,)> = sqlx::query_as(
        "SELECT id FROM clients WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL",
    )
    .bind(input.client_id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;
    if client_exists.is_none() {
        return Err(AppError::BadRequest("client not found in org".into()));
    }

    let next_run_at = compute_next_run_at(input.start_date, &input.frequency);
    let tax_rate = input.tax_rate.unwrap_or(Decimal::ZERO);
    let currency = input.currency.clone().unwrap_or_else(|| "USD".to_string());
    let line_items_json = serde_json::to_value(&input.line_items)
        .map_err(|e| AppError::internal(format!("line items: {e}")))?;

    let recurring: RecurringInvoice = sqlx::query_as(
        r#"
        INSERT INTO recurring_invoices (
            org_id, client_id, line_items, frequency, tax_rate, currency,
            start_date, end_date, next_run_at, notes
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        RETURNING id, org_id, client_id, line_items, frequency, tax_rate, currency,
                  status, start_date, end_date, next_run_at, last_run_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(tenant.org_id)
    .bind(input.client_id)
    .bind(&line_items_json)
    .bind(&input.frequency)
    .bind(tax_rate)
    .bind(&currency)
    .bind(input.start_date)
    .bind(input.end_date)
    .bind(next_run_at)
    .bind(&input.notes)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Created().json(recurring))
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

    let items: Vec<RecurringInvoice> = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, line_items, frequency, tax_rate, currency,
               status, start_date, end_date, next_run_at, last_run_at, notes,
               created_at, updated_at
        FROM recurring_invoices
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
    let r: Option<RecurringInvoice> = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, line_items, frequency, tax_rate, currency,
               status, start_date, end_date, next_run_at, last_run_at, notes,
               created_at, updated_at
        FROM recurring_invoices WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;
    r.map(|r| HttpResponse::Ok().json(r)).ok_or(AppError::NotFound)
}

#[patch("/{id}")]
async fn update(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
    input: web::Json<UpdateInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    if let Some(f) = input.frequency.as_deref() {
        validate_frequency(f)?;
    }
    let id = path.into_inner();

    let mut tx = pool.begin().await?;

    let current: RecurringInvoice = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, line_items, frequency, tax_rate, currency,
               status, start_date, end_date, next_run_at, last_run_at, notes,
               created_at, updated_at
        FROM recurring_invoices WHERE id = $1 AND org_id = $2 FOR UPDATE
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    if current.status == "cancelled" {
        return Err(AppError::BadRequest(
            "cannot update a cancelled recurring invoice".into(),
        ));
    }

    // If frequency changed, recompute next_run_at from start_date.
    let new_next_run = match &input.frequency {
        Some(f) if f != &current.frequency => Some(compute_next_run_at(current.start_date, f)),
        _ => current.next_run_at,
    };

    let line_items_json = match &input.line_items {
        Some(items) => Some(
            serde_json::to_value(items)
                .map_err(|e| AppError::internal(format!("line items: {e}")))?,
        ),
        None => None,
    };

    let updated: RecurringInvoice = sqlx::query_as(
        r#"
        UPDATE recurring_invoices SET
            line_items = COALESCE($3, line_items),
            frequency = COALESCE($4, frequency),
            tax_rate = COALESCE($5, tax_rate),
            currency = COALESCE($6, currency),
            end_date = COALESCE($7, end_date),
            notes = COALESCE($8, notes),
            next_run_at = $9,
            updated_at = now()
        WHERE id = $1 AND org_id = $2
        RETURNING id, org_id, client_id, line_items, frequency, tax_rate, currency,
                  status, start_date, end_date, next_run_at, last_run_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .bind(&line_items_json)
    .bind(&input.frequency)
    .bind(input.tax_rate)
    .bind(&input.currency)
    .bind(input.end_date)
    .bind(&input.notes)
    .bind(new_next_run)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(HttpResponse::Ok().json(updated))
}

#[post("/{id}/pause")]
async fn pause(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let r: Option<RecurringInvoice> = sqlx::query_as(
        r#"
        UPDATE recurring_invoices
        SET status = 'paused', updated_at = now()
        WHERE id = $1 AND org_id = $2 AND status = 'active'
        RETURNING id, org_id, client_id, line_items, frequency, tax_rate, currency,
                  status, start_date, end_date, next_run_at, last_run_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    r.map(|r| HttpResponse::Ok().json(r))
        .ok_or_else(|| AppError::BadRequest("only active recurring invoices can be paused".into()))
}

#[post("/{id}/resume")]
async fn resume(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();

    let mut tx = pool.begin().await?;
    let current: RecurringInvoice = sqlx::query_as(
        r#"
        SELECT id, org_id, client_id, line_items, frequency, tax_rate, currency,
               status, start_date, end_date, next_run_at, last_run_at, notes,
               created_at, updated_at
        FROM recurring_invoices WHERE id = $1 AND org_id = $2 FOR UPDATE
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(AppError::NotFound)?;

    if current.status != "paused" {
        return Err(AppError::BadRequest(
            "only paused recurring invoices can be resumed".into(),
        ));
    }

    let next_run_at = compute_next_run_at(current.start_date, &current.frequency);

    let updated: RecurringInvoice = sqlx::query_as(
        r#"
        UPDATE recurring_invoices
        SET status = 'active', next_run_at = $3, updated_at = now()
        WHERE id = $1 AND org_id = $2
        RETURNING id, org_id, client_id, line_items, frequency, tax_rate, currency,
                  status, start_date, end_date, next_run_at, last_run_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .bind(next_run_at)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(HttpResponse::Ok().json(updated))
}

#[post("/{id}/cancel")]
async fn cancel(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let r: Option<RecurringInvoice> = sqlx::query_as(
        r#"
        UPDATE recurring_invoices
        SET status = 'cancelled', next_run_at = NULL, updated_at = now()
        WHERE id = $1 AND org_id = $2 AND status != 'cancelled'
        RETURNING id, org_id, client_id, line_items, frequency, tax_rate, currency,
                  status, start_date, end_date, next_run_at, last_run_at, notes,
                  created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    r.map(|r| HttpResponse::Ok().json(r)).ok_or_else(|| {
        AppError::BadRequest("recurring invoice not found or already cancelled".into())
    })
}
