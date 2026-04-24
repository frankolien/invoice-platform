use actix_web::{HttpResponse, Responder, delete, get, patch, post, web};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::middleware::tenant::TenantContext;

pub const ALL_EVENTS: &[&str] = &[
    "invoice.created",
    "invoice.sent",
    "invoice.paid",
    "invoice.overdue",
    "invoice.cancelled",
    "payment.succeeded",
    "payment.failed",
];

#[derive(Debug, Deserialize, Validate)]
pub struct CreateInput {
    #[validate(url)]
    pub url: String,
    #[validate(length(min = 1))]
    pub events: Vec<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateInput {
    #[validate(url)]
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub status: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Subscription {
    pub id: Uuid,
    pub org_id: Uuid,
    pub url: String,
    pub events: Vec<String>,
    #[serde(skip_serializing)]
    pub secret: String,
    pub status: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Subscription {
    /// Variant that *includes* the secret — returned only on create.
    fn with_secret(&self) -> serde_json::Value {
        json!({
            "id": self.id,
            "org_id": self.org_id,
            "url": self.url,
            "events": self.events,
            "secret": self.secret,
            "status": self.status,
            "created_at": self.created_at,
            "updated_at": self.updated_at,
        })
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/webhook-subscriptions")
            .service(create)
            .service(list)
            .service(get_one)
            .service(update)
            .service(delete_one)
            .service(test_delivery),
    );
}

fn validate_events(events: &[String]) -> AppResult<()> {
    for e in events {
        if !ALL_EVENTS.contains(&e.as_str()) {
            return Err(AppError::BadRequest(format!("unknown event: {e}")));
        }
    }
    Ok(())
}

fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("whsec_{}", hex::encode(bytes))
}

#[post("")]
async fn create(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    input: web::Json<CreateInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    validate_events(&input.events)?;

    let secret = generate_secret();

    let sub: Subscription = sqlx::query_as(
        r#"
        INSERT INTO webhook_subscriptions (org_id, url, events, secret)
        VALUES ($1, $2, $3, $4)
        RETURNING id, org_id, url, events, secret, status, created_at, updated_at
        "#,
    )
    .bind(tenant.org_id)
    .bind(&input.url)
    .bind(&input.events)
    .bind(&secret)
    .fetch_one(pool.get_ref())
    .await?;

    // Only time we return the secret — the caller must store it now.
    Ok(HttpResponse::Created().json(sub.with_secret()))
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

    let items: Vec<Subscription> = sqlx::query_as(
        r#"
        SELECT id, org_id, url, events, secret, status, created_at, updated_at
        FROM webhook_subscriptions
        WHERE org_id = $1
        ORDER BY created_at DESC
        LIMIT $2 OFFSET $3
        "#,
    )
    .bind(tenant.org_id)
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
    let sub: Option<Subscription> = sqlx::query_as(
        r#"
        SELECT id, org_id, url, events, secret, status, created_at, updated_at
        FROM webhook_subscriptions WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    sub.map(|s| HttpResponse::Ok().json(s))
        .ok_or(AppError::NotFound)
}

#[patch("/{id}")]
async fn update(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
    input: web::Json<UpdateInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    if let Some(ref events) = input.events {
        validate_events(events)?;
    }
    if let Some(ref s) = input.status {
        if !matches!(s.as_str(), "active" | "inactive") {
            return Err(AppError::BadRequest("invalid status".into()));
        }
    }
    let id = path.into_inner();

    let sub: Option<Subscription> = sqlx::query_as(
        r#"
        UPDATE webhook_subscriptions SET
            url = COALESCE($3, url),
            events = COALESCE($4, events),
            status = COALESCE($5, status),
            updated_at = now()
        WHERE id = $1 AND org_id = $2
        RETURNING id, org_id, url, events, secret, status, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .bind(&input.url)
    .bind(&input.events)
    .bind(&input.status)
    .fetch_optional(pool.get_ref())
    .await?;

    sub.map(|s| HttpResponse::Ok().json(s))
        .ok_or(AppError::NotFound)
}

#[delete("/{id}")]
async fn delete_one(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let result = sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1 AND org_id = $2")
        .bind(id)
        .bind(tenant.org_id)
        .execute(pool.get_ref())
        .await?;
    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(HttpResponse::NoContent().finish())
}

#[post("/{id}/test")]
async fn test_delivery(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let sub: Option<Subscription> = sqlx::query_as(
        r#"
        SELECT id, org_id, url, events, secret, status, created_at, updated_at
        FROM webhook_subscriptions WHERE id = $1 AND org_id = $2
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    let sub = sub.ok_or(AppError::NotFound)?;

    let payload = json!({
        "event": "test",
        "data": { "message": "This is a test webhook delivery" },
        "timestamp": Utc::now().to_rfc3339(),
    })
    .to_string();

    match crate::jobs::webhook_delivery::deliver_once(
        &sub.url,
        &sub.secret,
        "test",
        &payload,
    )
    .await
    {
        Ok(status) => Ok(HttpResponse::Ok().json(json!({
            "delivered": (200..300).contains(&status),
            "status_code": status,
        }))),
        Err(e) => Ok(HttpResponse::Ok().json(json!({
            "delivered": false,
            "status_code": null,
            "error": e.to_string(),
        }))),
    }
}
