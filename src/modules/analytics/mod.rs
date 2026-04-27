//! Analytics endpoints — pure SQL aggregations over `payments` and `invoices`.
//!
//! Mirrors the TS app's two reports. The TS version routes reads at a Mongo
//! read replica; we don't have a separate replica here, so reads go to the
//! primary. If the workload ever justifies it, swap the pool for a read-only
//! replica pool — no change to the queries.

use actix_web::{HttpResponse, Responder, get, web};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::db::DbPool;
use crate::error::AppResult;
use crate::middleware::tenant::TenantContext;

#[derive(Debug, Deserialize)]
pub struct DateRangeQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
pub struct RevenueQuery {
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub currency: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RevenueReport {
    pub currency: String,
    pub total_revenue: Decimal,
    pub payment_count: i64,
    pub avg_payment: Decimal,
    pub monthly: Vec<MonthlyRevenue>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct MonthlyRevenue {
    pub year: i32,
    pub month: i32,
    pub total_revenue: Decimal,
    pub payment_count: i64,
    pub avg_payment: Decimal,
}

#[derive(Debug, Serialize)]
pub struct InvoiceReport {
    pub by_status: Vec<StatusGroup>,
    pub overdue: OverdueSummary,
}

#[derive(Debug, Serialize, FromRow)]
pub struct StatusGroup {
    pub status: String,
    pub count: i64,
    pub total_amount: Decimal,
}

#[derive(Debug, Serialize)]
pub struct OverdueSummary {
    pub count: i64,
    pub total_amount: Decimal,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/analytics")
            .service(revenue_report)
            .service(invoice_report),
    );
}

#[get("/revenue")]
async fn revenue_report(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    query: web::Query<RevenueQuery>,
) -> AppResult<impl Responder> {
    let currency = query.currency.clone().unwrap_or_else(|| "USD".to_string());

    // Monthly breakdown: succeeded payments grouped by year/month of paid_at.
    // Use date_part for portability across Postgres versions.
    let monthly: Vec<MonthlyRevenue> = sqlx::query_as(
        r#"
        SELECT
            date_part('year', paid_at)::int4 AS year,
            date_part('month', paid_at)::int4 AS month,
            ROUND(SUM(amount), 2) AS total_revenue,
            COUNT(*)::bigint AS payment_count,
            ROUND(AVG(amount), 2) AS avg_payment
        FROM payments
        WHERE org_id = $1
          AND status = 'succeeded'
          AND currency = $2
          AND paid_at IS NOT NULL
          AND ($3::timestamptz IS NULL OR paid_at >= $3)
          AND ($4::timestamptz IS NULL OR paid_at <= $4)
        GROUP BY year, month
        ORDER BY year DESC, month DESC
        "#,
    )
    .bind(tenant.org_id)
    .bind(&currency)
    .bind(query.from)
    .bind(query.to)
    .fetch_all(pool.get_ref())
    .await?;

    // Totals across the same window.
    let totals: (Option<Decimal>, i64, Option<Decimal>) = sqlx::query_as(
        r#"
        SELECT
            ROUND(SUM(amount), 2),
            COUNT(*)::bigint,
            ROUND(AVG(amount), 2)
        FROM payments
        WHERE org_id = $1
          AND status = 'succeeded'
          AND currency = $2
          AND paid_at IS NOT NULL
          AND ($3::timestamptz IS NULL OR paid_at >= $3)
          AND ($4::timestamptz IS NULL OR paid_at <= $4)
        "#,
    )
    .bind(tenant.org_id)
    .bind(&currency)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(RevenueReport {
        currency,
        total_revenue: totals.0.unwrap_or(Decimal::ZERO),
        payment_count: totals.1,
        avg_payment: totals.2.unwrap_or(Decimal::ZERO),
        monthly,
    }))
}

#[get("/invoices")]
async fn invoice_report(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    query: web::Query<DateRangeQuery>,
) -> AppResult<impl Responder> {
    // Invoice counts + totals grouped by status across the date window.
    let by_status: Vec<StatusGroup> = sqlx::query_as(
        r#"
        SELECT
            status,
            COUNT(*)::bigint AS count,
            ROUND(SUM(total), 2) AS total_amount
        FROM invoices
        WHERE org_id = $1
          AND ($2::timestamptz IS NULL OR created_at >= $2)
          AND ($3::timestamptz IS NULL OR created_at <= $3)
        GROUP BY status
        ORDER BY status
        "#,
    )
    .bind(tenant.org_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_all(pool.get_ref())
    .await?;

    // Overdue: sent / viewed / partially_paid past their due date.
    // Note: this scans live data, not just `status = 'overdue'`, so it picks
    // up invoices the cron hasn't transitioned yet.
    let overdue: (i64, Option<Decimal>) = sqlx::query_as(
        r#"
        SELECT
            COUNT(*)::bigint,
            ROUND(SUM(total), 2)
        FROM invoices
        WHERE org_id = $1
          AND status IN ('sent', 'viewed', 'partially_paid')
          AND due_date < now()
          AND ($2::timestamptz IS NULL OR created_at >= $2)
          AND ($3::timestamptz IS NULL OR created_at <= $3)
        "#,
    )
    .bind(tenant.org_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(InvoiceReport {
        by_status,
        overdue: OverdueSummary {
            count: overdue.0,
            total_amount: overdue.1.unwrap_or(Decimal::ZERO),
        },
    }))
}
