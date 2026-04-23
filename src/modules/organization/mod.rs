use actix_web::{HttpResponse, Responder, get, patch, post, web};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sqlx::FromRow;
use uuid::Uuid;
use validator::Validate;

use crate::db::DbPool;
use crate::error::{AppError, AppResult};
use crate::middleware::auth_user::AuthUser;

#[derive(Debug, Deserialize, Validate)]
pub struct CreateOrgInput {
    #[validate(length(min = 1, max = 120))]
    pub name: String,
    #[validate(length(min = 2, max = 80))]
    pub slug: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct UpdateOrgInput {
    #[validate(length(min = 1, max = 120))]
    pub name: Option<String>,
    pub plan: Option<String>,
    pub settings: Option<JsonValue>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct InviteInput {
    #[validate(email)]
    pub email: String,
    pub role: String,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Organization {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub owner_id: Uuid,
    pub plan: String,
    pub settings: JsonValue,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/organizations")
            .service(create)
            .service(get_one)
            .service(update)
            .service(invite),
    );
}

#[post("")]
async fn create(
    pool: web::Data<DbPool>,
    user: AuthUser,
    input: web::Json<CreateOrgInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let mut tx = pool.begin().await?;

    let org: Organization = sqlx::query_as(
        r#"
        INSERT INTO organizations (name, slug, owner_id)
        VALUES ($1, $2, $3)
        RETURNING id, name, slug, owner_id, plan, settings, created_at, updated_at
        "#,
    )
    .bind(&input.name)
    .bind(&input.slug)
    .bind(user.user_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
            AppError::Conflict("slug already taken".into())
        }
        other => AppError::from(other),
    })?;

    sqlx::query(
        "INSERT INTO members (user_id, org_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(user.user_id)
    .bind(org.id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(HttpResponse::Created().json(org))
}

#[get("/{id}")]
async fn get_one(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
) -> AppResult<impl Responder> {
    let id = path.into_inner();

    let membership: Option<(i64,)> =
        sqlx::query_as("SELECT 1::bigint FROM members WHERE user_id = $1 AND org_id = $2")
            .bind(user.user_id)
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await?;
    if membership.is_none() {
        return Err(AppError::Forbidden);
    }

    let org: Option<Organization> = sqlx::query_as(
        "SELECT id, name, slug, owner_id, plan, settings, created_at, updated_at FROM organizations WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool.get_ref())
    .await?;

    org.map(|o| HttpResponse::Ok().json(o))
        .ok_or(AppError::NotFound)
}

#[patch("/{id}")]
async fn update(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
    input: web::Json<UpdateOrgInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    let id = path.into_inner();

    let role: Option<(String,)> =
        sqlx::query_as("SELECT role FROM members WHERE user_id = $1 AND org_id = $2")
            .bind(user.user_id)
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await?;
    let role = role.ok_or(AppError::Forbidden)?.0;
    if role != "owner" && role != "admin" {
        return Err(AppError::Forbidden);
    }

    let org: Organization = sqlx::query_as(
        r#"
        UPDATE organizations
        SET name = COALESCE($2, name),
            plan = COALESCE($3, plan),
            settings = COALESCE($4, settings),
            updated_at = now()
        WHERE id = $1
        RETURNING id, name, slug, owner_id, plan, settings, created_at, updated_at
        "#,
    )
    .bind(id)
    .bind(&input.name)
    .bind(&input.plan)
    .bind(&input.settings)
    .fetch_one(pool.get_ref())
    .await?;

    Ok(HttpResponse::Ok().json(org))
}

#[post("/{id}/invite")]
async fn invite(
    pool: web::Data<DbPool>,
    user: AuthUser,
    path: web::Path<Uuid>,
    input: web::Json<InviteInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;
    if !matches!(input.role.as_str(), "admin" | "accountant" | "viewer") {
        return Err(AppError::BadRequest("invalid role".into()));
    }
    let id = path.into_inner();

    let role: Option<(String,)> =
        sqlx::query_as("SELECT role FROM members WHERE user_id = $1 AND org_id = $2")
            .bind(user.user_id)
            .bind(id)
            .fetch_optional(pool.get_ref())
            .await?;
    let role = role.ok_or(AppError::Forbidden)?.0;
    if role != "owner" && role != "admin" {
        return Err(AppError::Forbidden);
    }

    let invitee: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&input.email)
        .fetch_optional(pool.get_ref())
        .await?;
    let invitee_id = invitee.ok_or_else(|| AppError::NotFound)?.0;

    sqlx::query(
        "INSERT INTO members (user_id, org_id, role) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, org_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(invitee_id)
    .bind(id)
    .bind(&input.role)
    .execute(pool.get_ref())
    .await?;

    Ok(HttpResponse::NoContent().finish())
}
