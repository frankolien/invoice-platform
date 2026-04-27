pub mod auth;
pub mod cache;
pub mod config;
pub mod db;
pub mod error;
pub mod jobs;
pub mod middleware;
pub mod modules;
pub mod observability;

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceFactory, ServiceRequest, ServiceResponse};
use actix_web::{App, web};

use crate::auth::jwt::TokenService;
use crate::cache::Cache;
use crate::config::Config;
use crate::db::DbPool;
use crate::jobs::JobQueues;
use crate::modules::payment::stripe_client::StripeClient;
use crate::observability::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub pool: web::Data<DbPool>,
    pub tokens: web::Data<TokenService>,
    pub metrics: web::Data<Metrics>,
    pub cache: web::Data<Cache>,
    pub config: web::Data<Config>,
    pub stripe: Option<web::Data<StripeClient>>,
    pub queues: web::Data<JobQueues>,
}

impl AppState {
    pub fn new(
        pool: DbPool,
        tokens: TokenService,
        cache: Cache,
        config: Config,
        stripe: Option<StripeClient>,
        queues: JobQueues,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            pool: web::Data::new(pool),
            tokens: web::Data::new(tokens),
            metrics: web::Data::new(Metrics::new()?),
            cache: web::Data::new(cache),
            config: web::Data::new(config),
            stripe: stripe.map(web::Data::new),
            queues: web::Data::new(queues),
        })
    }
}

pub fn build_app(
    state: AppState,
) -> App<
    impl ServiceFactory<
        ServiceRequest,
        Config = (),
        Response = ServiceResponse<impl MessageBody>,
        Error = actix_web::Error,
        InitError = (),
    >,
> {
    let mut app = App::new()
        .app_data(state.pool)
        .app_data(state.tokens)
        .app_data(state.metrics)
        .app_data(state.cache)
        .app_data(state.config)
        .app_data(state.queues)
        .app_data(web::JsonConfig::default().limit(1024 * 1024))
        .app_data(web::PayloadConfig::default().limit(1024 * 1024));

    if let Some(stripe) = state.stripe {
        app = app.app_data(stripe);
    }

    app.configure(observability::health::configure)
        .configure(observability::metrics::configure)
        .service(
            // Only /v1 is rate-limited + audited. Health, metrics, and root
            // stay unwrapped so probes never get 429'd and never log audit
            // noise.
            web::scope("/v1")
                .wrap(actix_web::middleware::from_fn(
                    middleware::audit_log::audit_log,
                ))
                .wrap(actix_web::middleware::from_fn(
                    middleware::rate_limit::rate_limit,
                ))
                .configure(modules::auth::configure)
                .configure(modules::organization::configure)
                .configure(modules::client::configure)
                .configure(modules::invoice::configure)
                .configure(modules::payment::configure)
                .configure(modules::payment::webhook::configure)
                .configure(modules::webhook_subscription::configure)
                .configure(modules::recurring_invoice::configure)
                .configure(modules::analytics::configure),
        )
}
