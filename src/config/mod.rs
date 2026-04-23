use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub env: String,
    pub database_url: String,
    pub jwt_secret: String,
    pub jwt_refresh_secret: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let _ = dotenvy::dotenv();

        Ok(Self {
            port: env::var("PORT").unwrap_or_else(|_| "3000".into()).parse()?,
            env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            database_url: env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL is required"))?,
            jwt_secret: env::var("JWT_SECRET")
                .map_err(|_| anyhow::anyhow!("JWT_SECRET is required"))?,
            jwt_refresh_secret: env::var("JWT_REFRESH_SECRET")
                .map_err(|_| anyhow::anyhow!("JWT_REFRESH_SECRET is required"))?,
            access_token_ttl_secs: env::var("ACCESS_TOKEN_TTL_SECS")
                .unwrap_or_else(|_| "900".into())
                .parse()?,
            refresh_token_ttl_secs: env::var("REFRESH_TOKEN_TTL_SECS")
                .unwrap_or_else(|_| "604800".into())
                .parse()?,
        })
    }
}
