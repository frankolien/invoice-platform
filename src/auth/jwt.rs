use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,
    pub email: String,
    pub exp: i64,
    pub iat: i64,
    pub typ: TokenType,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
    Access,
    Refresh,
}

pub struct TokenService {
    access_secret: String,
    refresh_secret: String,
    access_ttl: Duration,
    refresh_ttl: Duration,
}

impl TokenService {
    pub fn new(
        access_secret: String,
        refresh_secret: String,
        access_ttl_secs: i64,
        refresh_ttl_secs: i64,
    ) -> Self {
        Self {
            access_secret,
            refresh_secret,
            access_ttl: Duration::seconds(access_ttl_secs),
            refresh_ttl: Duration::seconds(refresh_ttl_secs),
        }
    }

    pub fn sign_access(&self, user_id: Uuid, email: &str) -> AppResult<String> {
        self.sign(user_id, email, TokenType::Access, self.access_ttl, &self.access_secret)
    }

    pub fn sign_refresh(&self, user_id: Uuid, email: &str) -> AppResult<String> {
        self.sign(user_id, email, TokenType::Refresh, self.refresh_ttl, &self.refresh_secret)
    }

    pub fn verify_access(&self, token: &str) -> AppResult<Claims> {
        self.verify(token, TokenType::Access, &self.access_secret)
    }

    pub fn verify_refresh(&self, token: &str) -> AppResult<Claims> {
        self.verify(token, TokenType::Refresh, &self.refresh_secret)
    }

    fn sign(
        &self,
        user_id: Uuid,
        email: &str,
        typ: TokenType,
        ttl: Duration,
        secret: &str,
    ) -> AppResult<String> {
        let now = Utc::now();
        let claims = Claims {
            sub: user_id,
            email: email.to_string(),
            iat: now.timestamp(),
            exp: (now + ttl).timestamp(),
            typ,
        };
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .map_err(AppError::from)
    }

    fn verify(&self, token: &str, expected: TokenType, secret: &str) -> AppResult<Claims> {
        let data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(secret.as_bytes()),
            &Validation::default(),
        )?;
        if data.claims.typ != expected {
            return Err(AppError::Unauthorized);
        }
        Ok(data.claims)
    }
}
