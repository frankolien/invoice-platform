use std::future::Future;
use std::pin::Pin;

use actix_web::{FromRequest, HttpRequest, dev::Payload, web};
use uuid::Uuid;

use crate::db::DbPool;
use crate::error::AppError;
use crate::middleware::auth_user::AuthUser;

#[derive(Debug, Clone)]
pub struct TenantContext {
    pub user_id: Uuid,
    pub org_id: Uuid,
    pub role: String,
}

impl FromRequest for TenantContext {
    type Error = AppError;
    type Future = Pin<Box<dyn Future<Output = Result<Self, Self::Error>>>>;

    fn from_request(req: &HttpRequest, payload: &mut Payload) -> Self::Future {
        let auth_future = AuthUser::from_request(req, payload);
        let pool = req.app_data::<web::Data<DbPool>>().cloned();
        let org_header = req
            .headers()
            .get("x-org-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Box::pin(async move {
            let user = auth_future.await?;
            let pool = pool.ok_or_else(|| AppError::internal("db pool missing"))?;
            let org_header = org_header.ok_or_else(|| {
                AppError::BadRequest("missing x-org-id header".into())
            })?;
            let org_id: Uuid = org_header
                .parse()
                .map_err(|_| AppError::BadRequest("invalid x-org-id".into()))?;

            let row: Option<(String,)> = sqlx::query_as(
                "SELECT role FROM members WHERE user_id = $1 AND org_id = $2",
            )
            .bind(user.user_id)
            .bind(org_id)
            .fetch_optional(pool.get_ref())
            .await?;

            let role = row.ok_or(AppError::Forbidden)?.0;

            Ok(TenantContext {
                user_id: user.user_id,
                org_id,
                role,
            })
        })
    }
}
