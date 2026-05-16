use actix_web::{HttpResponse, Responder, post, web};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::auth::{jwt::TokenService, password};
use crate::db::DbPool;
use crate::error::{AppError, AppResult};

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct RegisterInput {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 8, max = 128))]
    pub password: String,
    #[validate(length(min = 1, max = 120))]
    pub name: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
pub struct LoginInput {
    #[validate(email)]
    pub email: String,
    #[validate(length(min = 1))]
    pub password: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshInput {
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AuthResponse {
    pub user: UserDto,
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UserDto {
    pub id: Uuid,
    pub email: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
}

#[derive(FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    password_hash: String,
    name: String,
    created_at: DateTime<Utc>,
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::scope("/auth")
            .service(register)
            .service(login)
            .service(refresh_tokens),
    );
}

#[utoipa::path(
    post,
    path = "/v1/auth/register",
    tag = "auth",
    request_body = RegisterInput,
    responses(
        (status = 201, description = "User registered", body = AuthResponse),
        (status = 400, description = "Validation error"),
        (status = 409, description = "Email already registered"),
    )
)]
#[post("/register")]
pub async fn register(
    pool: web::Data<DbPool>,
    tokens: web::Data<TokenService>,
    input: web::Json<RegisterInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let existing: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM users WHERE email = $1")
        .bind(&input.email)
        .fetch_optional(pool.get_ref())
        .await?;
    if existing.is_some() {
        return Err(AppError::Conflict("email already registered".into()));
    }

    let hash = password::hash(&input.password).await?;

    let row: UserRow = sqlx::query_as(
        r#"
        INSERT INTO users (email, password_hash, name)
        VALUES ($1, $2, $3)
        RETURNING id, email, password_hash, name, created_at
        "#,
    )
    .bind(&input.email)
    .bind(&hash)
    .bind(&input.name)
    .fetch_one(pool.get_ref())
    .await?;

    let access_token = tokens.sign_access(row.id, &row.email)?;
    let refresh_token = tokens.sign_refresh(row.id, &row.email)?;

    Ok(HttpResponse::Created().json(AuthResponse {
        user: UserDto {
            id: row.id,
            email: row.email,
            name: row.name,
            created_at: row.created_at,
        },
        access_token,
        refresh_token,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/auth/login",
    tag = "auth",
    request_body = LoginInput,
    responses(
        (status = 200, description = "Logged in", body = AuthResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "Invalid credentials"),
    )
)]
#[post("/login")]
pub async fn login(
    pool: web::Data<DbPool>,
    tokens: web::Data<TokenService>,
    input: web::Json<LoginInput>,
) -> AppResult<impl Responder> {
    input.validate().map_err(|e| AppError::Validation(e.to_string()))?;

    let row: Option<UserRow> = sqlx::query_as(
        "SELECT id, email, password_hash, name, created_at FROM users WHERE email = $1",
    )
    .bind(&input.email)
    .fetch_optional(pool.get_ref())
    .await?;

    let row = row.ok_or(AppError::Unauthorized)?;

    if !password::verify(&input.password, &row.password_hash).await? {
        return Err(AppError::Unauthorized);
    }

    let access_token = tokens.sign_access(row.id, &row.email)?;
    let refresh_token = tokens.sign_refresh(row.id, &row.email)?;

    Ok(HttpResponse::Ok().json(AuthResponse {
        user: UserDto {
            id: row.id,
            email: row.email,
            name: row.name,
            created_at: row.created_at,
        },
        access_token,
        refresh_token,
    }))
}

#[utoipa::path(
    post,
    path = "/v1/auth/refresh",
    tag = "auth",
    request_body = RefreshInput,
    responses(
        (status = 200, description = "Tokens refreshed", body = TokenPair),
        (status = 401, description = "Invalid refresh token"),
    )
)]
#[post("/refresh")]
pub async fn refresh_tokens(
    tokens: web::Data<TokenService>,
    input: web::Json<RefreshInput>,
) -> AppResult<impl Responder> {
    let claims = tokens.verify_refresh(&input.refresh_token)?;
    let access_token = tokens.sign_access(claims.sub, &claims.email)?;
    let refresh_token = tokens.sign_refresh(claims.sub, &claims.email)?;
    Ok(HttpResponse::Ok().json(serde_json::json!({
        "access_token": access_token,
        "refresh_token": refresh_token,
    })))
}
