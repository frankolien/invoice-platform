use actix_web::HttpServer;
use tracing_actix_web::TracingLogger;

use invoice_platform::auth::jwt::TokenService;
use invoice_platform::config::Config;
use invoice_platform::observability::init_tracing;
use invoice_platform::{AppState, build_app, db};

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();

    let cfg = Config::from_env()?;
    tracing::info!(port = cfg.port, env = %cfg.env, "starting invoice-platform");

    let pool = db::connect(&cfg.database_url).await?;
    tracing::info!("postgres connected, migrations applied");

    let tokens = TokenService::new(
        cfg.jwt_secret.clone(),
        cfg.jwt_refresh_secret.clone(),
        cfg.access_token_ttl_secs,
        cfg.refresh_token_ttl_secs,
    );

    let state = AppState::new(pool, tokens)?;
    let port = cfg.port;

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
