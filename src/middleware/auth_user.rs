use std::future::{Ready, ready};

use actix_web::{FromRequest, HttpRequest, dev::Payload, web};
use uuid::Uuid;

use crate::auth::jwt::TokenService;
use crate::error::AppError;

#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub email: String,
}

impl FromRequest for AuthUser {
    type Error = AppError;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut Payload) -> Self::Future {
        let tokens = match req.app_data::<web::Data<TokenService>>() {
            Some(t) => t.clone(),
            None => return ready(Err(AppError::internal("token service missing"))),
        };

        let header = req
            .headers()
            .get(actix_web::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok());

        let token = match header.and_then(|h| h.strip_prefix("Bearer ")) {
            Some(t) => t.trim(),
            None => return ready(Err(AppError::Unauthorized)),
        };

        ready(match tokens.verify_access(token) {
            Ok(claims) => Ok(AuthUser {
                user_id: claims.sub,
                email: claims.email,
            }),
            Err(_) => Err(AppError::Unauthorized),
        })
    }
}
