use actix_web::HttpServer;
use tracing_actix_web::TracingLogger;

use invoice_platform::auth::jwt::TokenService;
use invoice_platform::cache::Cache;
use invoice_platform::circuit_breaker::CircuitBreakers;
use invoice_platform::config::Config;
use invoice_platform::jobs;
use invoice_platform::modules::payment::stripe_client::StripeClient;
use invoice_platform::observability::init_tracing;
use invoice_platform::{AppState, build_app, db};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let tracer_provider = init_tracing(cfg.otel_endpoint.as_deref());
    if cfg.otel_endpoint.is_some() {
        tracing::info!(
            endpoint = cfg.otel_endpoint.as_deref(),
            "otel exporter enabled"
        );
    }

    tracing::info!(port = cfg.port, env = %cfg.env, "starting invoice-platform");

    let pool = db::connect(&cfg.database_url).await?;
    tracing::info!("postgres connected, migrations applied");

    let cache = Cache::connect(&cfg.redis_url).await?;
    tracing::info!("redis connected");

    let queues = jobs::connect(&cfg.redis_url).await?;
    let breakers = CircuitBreakers::new();
    jobs::spawn_workers(
        queues.clone(),
        pool.clone(),
        cfg.app_url.clone(),
        breakers.clone(),
    );
    tracing::info!("job workers spawned");

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
    let mut state = AppState::new(pool, tokens, cache, cfg, stripe, queues)?;
    // Replace the AppState's default breakers with the shared instance we
    // also gave to the workers, so HTTP + workers see the same state.
    state.breakers = actix_web::web::Data::new(breakers);

    let server = HttpServer::new(move || build_app(state.clone()).wrap(TracingLogger::default()))
        .workers(20)
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

    // Drain spans to the OTel collector before exit. Important — without
    // this, the last batch of spans can be lost on shutdown.
    if let Some(p) = tracer_provider {
        if let Err(e) = p.shutdown() {
            tracing::warn!(error = %e, "otel tracer shutdown");
        }
    }

    tracing::info!("shutdown complete");
    Ok(())
}
