mod auth;
mod config;
mod db;
mod error;
mod middleware;
mod modules;
mod observability;

use actix_web::{App, HttpServer, web};
use tracing_actix_web::TracingLogger;

use crate::auth::jwt::TokenService;
use crate::config::Config;
use crate::observability::{health, init_tracing, metrics::Metrics};

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
    let metrics = Metrics::new()?;

    let pool_data = web::Data::new(pool);
    let tokens_data = web::Data::new(tokens);
    let metrics_data = web::Data::new(metrics);
    let port = cfg.port;

    let server = HttpServer::new(move || {
        App::new()
            .app_data(pool_data.clone())
            .app_data(tokens_data.clone())
            .app_data(metrics_data.clone())
            .app_data(web::JsonConfig::default().limit(1024 * 1024))
            .wrap(TracingLogger::default())
            .configure(health::configure)
            .configure(observability::metrics::configure)
            .service(
                web::scope("/v1")
                    .configure(modules::auth::configure)
                    .configure(modules::organization::configure)
                    .configure(modules::client::configure)
                    .configure(modules::invoice::configure),
            )
    })
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
