pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod middleware;
pub mod modules;
pub mod observability;

use actix_web::body::MessageBody;
use actix_web::dev::{ServiceFactory, ServiceRequest, ServiceResponse};
use actix_web::{App, web};

use crate::auth::jwt::TokenService;
use crate::db::DbPool;
use crate::observability::metrics::Metrics;

#[derive(Clone)]
pub struct AppState {
    pub pool: web::Data<DbPool>,
    pub tokens: web::Data<TokenService>,
    pub metrics: web::Data<Metrics>,
}

impl AppState {
    pub fn new(pool: DbPool, tokens: TokenService) -> anyhow::Result<Self> {
        Ok(Self {
            pool: web::Data::new(pool),
            tokens: web::Data::new(tokens),
            metrics: web::Data::new(Metrics::new()?),
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
    App::new()
        .app_data(state.pool)
        .app_data(state.tokens)
        .app_data(state.metrics)
        .app_data(web::JsonConfig::default().limit(1024 * 1024))
        .configure(observability::health::configure)
        .configure(observability::metrics::configure)
        .service(
            web::scope("/v1")
                .configure(modules::auth::configure)
                .configure(modules::organization::configure)
                .configure(modules::client::configure)
                .configure(modules::invoice::configure),
        )
}
