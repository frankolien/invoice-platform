use std::env;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub env: String,
    pub database_url: String,
    pub redis_url: String,
    pub app_url: String,
    pub jwt_secret: String,
    pub jwt_refresh_secret: String,
    pub access_token_ttl_secs: i64,
    pub refresh_token_ttl_secs: i64,
    pub stripe_secret_key: Option<String>,
    pub stripe_webhook_secret: Option<String>,
    /// OTLP HTTP endpoint for span export (Jaeger / OTel collector). When
    /// `None` (env var unset or empty), tracing stays log-only.
    pub otel_endpoint: Option<String>,
}

impl Config {
    pub fn from_env() -> anyhow::Result<Self> {
        let _ = dotenvy::dotenv();

        Ok(Self {
            port: env::var("PORT").unwrap_or_else(|_| "3000".into()).parse()?,
            env: env::var("APP_ENV").unwrap_or_else(|_| "development".into()),
            database_url: env::var("DATABASE_URL")
                .map_err(|_| anyhow::anyhow!("DATABASE_URL is required"))?,
            redis_url: env::var("REDIS_URL")
                .unwrap_or_else(|_| "redis://localhost:6379".into()),
            app_url: env::var("APP_URL").unwrap_or_else(|_| "http://localhost:3000".into()),
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
            stripe_secret_key: env::var("STRIPE_SECRET_KEY").ok().filter(|s| !s.is_empty()),
            stripe_webhook_secret: env::var("STRIPE_WEBHOOK_SECRET")
                .ok()
                .filter(|s| !s.is_empty()),
            otel_endpoint: env::var("OTEL_EXPORTER_OTLP_ENDPOINT")
                .ok()
                .filter(|s| !s.is_empty()),
        })
    }
}
