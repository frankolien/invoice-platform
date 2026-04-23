use actix_web::{HttpResponse, Responder, get, web};
use serde_json::json;

use crate::db::DbPool;

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.service(health).service(ready).service(root);
}

#[get("/health")]
async fn health(pool: web::Data<DbPool>) -> impl Responder {
    let db_ok = sqlx::query("SELECT 1").execute(pool.get_ref()).await.is_ok();
    let status = if db_ok { "ok" } else { "degraded" };
    let code = if db_ok { 200 } else { 503 };
    HttpResponse::build(actix_web::http::StatusCode::from_u16(code).unwrap())
        .json(json!({ "status": status, "postgres": db_ok }))
}

#[get("/ready")]
async fn ready(pool: web::Data<DbPool>) -> impl Responder {
    match sqlx::query("SELECT 1").execute(pool.get_ref()).await {
        Ok(_) => HttpResponse::Ok().json(json!({ "ready": true })),
        Err(_) => HttpResponse::ServiceUnavailable().json(json!({ "ready": false })),
    }
}

#[get("/")]
async fn root() -> impl Responder {
    HttpResponse::Ok().json(json!({
        "name": "invoice-platform",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}
