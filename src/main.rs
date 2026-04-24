use actix_web::HttpServer;
use tracing_actix_web::TracingLogger;

use invoice_platform::auth::jwt::TokenService;
use invoice_platform::cache::Cache;
use invoice_platform::config::Config;
use invoice_platform::modules::payment::stripe_client::StripeClient;
use invoice_platform::observability::init_tracing;
use invoice_platform::{AppState, build_app, db};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env()?;
    tracing::info!(port = cfg.port, env = %cfg.env, "starting invoice-platform");

    let pool = db::connect(&cfg.database_url).await?;
    tracing::info!("postgres connected, migrations applied");

    let cache = Cache::connect(&cfg.redis_url).await?;
    tracing::info!("redis connected");

    let stripe = match (&cfg.stripe_secret_key, &cfg.stripe_webhook_secret) {
        (Some(sk), Some(ws)) => {
            tracing::info!("stripe configured");
            Some(StripeClient::new(sk, ws))
        }
        _ => {
            tracing::warn!("stripe not configured — /pay, /refund, /webhooks/stripe disabled");
            None
        }
    };

    let tokens = TokenService::new(
        cfg.jwt_secret.clone(),
        cfg.jwt_refresh_secret.clone(),
        cfg.access_token_ttl_secs,
        cfg.refresh_token_ttl_secs,
    );

    let port = cfg.port;
    let state = AppState::new(pool, tokens, cache, cfg, stripe)?;

    let server = HttpServer::new(move || build_app(state.clone()).wrap(TracingLogger::default()))
        .bind(("0.0.0.0", port))?
        .shutdown_timeout(15)
        .run();

    let handle = server.handle();

    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            tracing::info!("shutdown signal received, draining...");
            handle.stop(true).await;
        }
    });

    server.await?;
    tracing::info!("shutdown complete");
    Ok(())
}
