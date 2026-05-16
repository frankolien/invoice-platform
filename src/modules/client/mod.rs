use actix_web::{HttpResponse, Responder, delete, get, patch, post, web};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;
use validator::Validate;

use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::middleware::tenant::TenantContext;

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct CreateClientInput {
    #[validate(length(min = 1, max = 200))]
    pub name: String,
    #[validate(email)]
    pub email: String,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct UpdateClientInput {
    #[validate(length(min = 1, max = 200))]
    pub name: Option<String>,
    #[validate(email)]
    pub email: Option<String>,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListQuery {
    pub page: Option<i64>,
    pub page_size: Option<i64>,
    pub search: Option<String>,
}

#[derive(Debug, Serialize, FromRow, ToSchema)]
pub struct Client {
    pub id: Uuid,
    pub org_id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub address: Option<String>,
    pub tax_id: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ClientPage {
    pub items: Vec<Client>,
    pub page: i64,
    pub page_size: i64,
    pub total: i64,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/clients")
            .service(create)
            .service(list)
            .service(get_one)
            .service(update)
            .service(soft_delete),
    );
}

#[utoipa::path(
    post,
    path = "/v1/clients",
    tag = "clients",
    request_body = CreateClientInput,
    security(("bearer_auth" = [])),
    responses(
        (status = 201, description = "Client created", body = Client),
        (status = 400, description = "Validation error"),
    )
)]
#[post("")]
pub async fn create(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    input: web::Json<CreateClientInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let client: Client = sqlx::query_as(
        r#"
        INSERT INTO clients (org_id, name, email, phone, address, tax_id, notes)
        VALUES ($1, $2, $3, $4, $5, $6, $7)
        RETURNING id, org_id, name, email, phone, address, tax_id, notes, created_at, updated_at
        "#,
    )
    .bind(tenant.org_id)
    .bind(&input.name)
    .bind(&input.email)
    .bind(&input.phone)
    .bind(&input.address)
    .bind(&input.tax_id)
    .bind(&input.notes)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Created().json(client))
}

#[utoipa::path(
    get,
    path = "/v1/clients",
    tag = "clients",
    params(ListQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Paginated clients", body = ClientPage),
    )
)]
#[get("")]
pub async fn list(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    query: web::Query<ListQuery>,
) -> AppResult<impl Responder> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = (page - 1) * page_size;
    let search = query.search.as_deref().unwrap_or("").trim();
    let like = format!("%{search}%");

    let items: Vec<Client> = sqlx::query_as(
        r#"
        SELECT id, org_id, name, email, phone, address, tax_id, notes, created_at, updated_at
        FROM clients
        WHERE org_id = $1
          AND deleted_at IS NULL
          AND ($2 = '' OR name ILIKE $3 OR email ILIKE $3)
        ORDER BY created_at DESC
        LIMIT $4 OFFSET $5
        "#,
    )
    .bind(tenant.org_id)
    .bind(search)
    .bind(&like)
    .bind(page_size)
    .bind(offset)
    .fetch_all(pool.get_ref())
    .await?;

    let total: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)::bigint FROM clients
        WHERE org_id = $1 AND deleted_at IS NULL
          AND ($2 = '' OR name ILIKE $3 OR email ILIKE $3)
        "#,
    )
    .bind(tenant.org_id)
    .bind(search)
    .bind(&like)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(ClientPage {
        items,
        page,
        page_size,
        total: total.0,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/clients/{id}",
    tag = "clients",
    params(("id" = Uuid, Path, description = "Client id")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Client", body = Client),
        (status = 404, description = "Not found"),
    )
)]
#[get("/{id}")]
pub async fn get_one(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let client: Option<Client> = sqlx::query_as(
        r#"
        SELECT id, org_id, name, email, phone, address, tax_id, notes, created_at, updated_at
        FROM clients
        WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .fetch_optional(pool.get_ref())
    .await?;

    client
        .map(|c| HttpResponse::Ok().json(c))
        .ok_or(AppError::NotFound)
}

#[utoipa::path(
    patch,
    path = "/v1/clients/{id}",
    tag = "clients",
    params(("id" = Uuid, Path, description = "Client id")),
    request_body = UpdateClientInput,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Client updated", body = Client),
        (status = 404, description = "Not found"),
    )
)]
#[patch("/{id}")]
pub async fn update(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
    input: web::Json<UpdateClientInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    let id = path.into_inner();

    let client: Option<Client> = sqlx::query_as(
        r#"
        UPDATE clients SET
            name = COALESCE($3, name),
            email = COALESCE($4, email),
            phone = COALESCE($5, phone),
            address = COALESCE($6, address),
            tax_id = COALESCE($7, tax_id),
            notes = COALESCE($8, notes),
            updated_at = now()
        WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL
        RETURNING id, org_id, name, email, phone, address, tax_id, notes, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(tenant.org_id)
    .bind(&input.name)
    .bind(&input.email)
    .bind(&input.phone)
    .bind(&input.address)
    .bind(&input.tax_id)
    .bind(&input.notes)
    .fetch_optional(pool.get_ref())
    .await?;

    client
        .map(|c| HttpResponse::Ok().json(c))
        .ok_or(AppError::NotFound)
}

#[utoipa::path(
    delete,
    path = "/v1/clients/{id}",
    tag = "clients",
    params(("id" = Uuid, Path, description = "Client id")),
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Client soft-deleted"),
        (status = 404, description = "Not found"),
    )
)]
#[delete("/{id}")]
pub async fn soft_delete(
    pool: web::Data<DbPool>,
    tenant: TenantContext,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();
    let result = sqlx::query(
        "UPDATE clients SET deleted_at = now() WHERE id = $1 AND org_id = $2 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(tenant.org_id)
    .execute(pool.get_ref())
    .await?;

    if result.rows_affected() == 0 {
        return Err(AppError::NotFound);
    }
    Ok(HttpResponse::NoContent().finish())
}
