use actix_http::Request;
use actix_web::body::MessageBody;
use actix_web::dev::{Service, ServiceResponse};
use actix_web::{http::StatusCode, test};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use invoice_platform::auth::jwt::TokenService;
use invoice_platform::cache::Cache;
use invoice_platform::config::Config;
use invoice_platform::jobs;
use invoice_platform::{AppState, build_app};

pub const TEST_ACCESS_SECRET: &str = "test-access-secret";
pub const TEST_REFRESH_SECRET: &str = "test-refresh-secret";

pub fn token_service() -> TokenService {
    TokenService::new(
        TEST_ACCESS_SECRET.into(),
        TEST_REFRESH_SECRET.into(),
        900,
        604_800,
    )
}

fn test_config() -> Config {
    Config {
        port: 0,
        env: "test".into(),
        database_url: String::new(),
        redis_url: std::env::var("REDIS_URL")
            .unwrap_or_else(|_| "redis://localhost:6380".into()),
        app_url: "http://localhost:3000".into(),
        jwt_secret: TEST_ACCESS_SECRET.into(),
        jwt_refresh_secret: TEST_REFRESH_SECRET.into(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604_800,
        stripe_secret_key: None,
        stripe_webhook_secret: None,
    }
}

pub async fn make_app(
    pool: PgPool,
) -> impl Service<Request, Response = ServiceResponse<impl MessageBody>, Error = actix_web::Error>
{
    let cfg = test_config();
    let cache = Cache::connect(&cfg.redis_url)
        .await
        .expect("connect redis (is docker compose up?)");
    let queues = jobs::connect(&cfg.redis_url)
        .await
        .expect("connect apalis redis");
    // Note: we intentionally don't spawn workers in tests — enqueues land in
    // Redis but nothing drains them. The invoice /send endpoint succeeds
    // whether or not the email job runs.
    let state =
        AppState::new(pool, token_service(), cache, cfg, None, queues).expect("build state");
    test::init_service(build_app(state)).await
}

pub struct SeededUser {
    pub id: Uuid,
    pub email: String,
    pub access_token: String,
}

pub async fn seed_user<S, B>(app: &S, name_prefix: &str) -> SeededUser
where
    S: Service<Request, Response = ServiceResponse<B>, Error = actix_web::Error>,
    B: MessageBody,
{
    let email = format!("{}-{}@test.local", name_prefix, Uuid::new_v4());
    let req = test::TestRequest::post()
        .uri("/v1/auth/register")
        .set_json(json!({
            "email": email,
            "password": "password123",
            "name": name_prefix,
        }))
        .to_request();
    let resp = test::call_service(app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED, "register failed");
    let body: Value = test::read_body_json(resp).await;
    SeededUser {
        id: body["user"]["id"].as_str().unwrap().parse().unwrap(),
        email,
        access_token: body["access_token"].as_str().unwrap().to_string(),
    }
}

pub struct SeededOrg {
    pub id: Uuid,
    pub slug: String,
}

pub async fn seed_org<S, B>(app: &S, token: &str, slug_prefix: &str) -> SeededOrg
where
    S: Service<Request, Response = ServiceResponse<B>, Error = actix_web::Error>,
    B: MessageBody,
{
    let slug = format!("{}-{}", slug_prefix, &Uuid::new_v4().to_string()[..8]);
    let req = test::TestRequest::post()
        .uri("/v1/organizations")
        .insert_header(("authorization", format!("Bearer {token}")))
        .set_json(json!({ "name": slug_prefix, "slug": slug }))
        .to_request();
    let resp = test::call_service(app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED, "create org failed");
    let body: Value = test::read_body_json(resp).await;
    SeededOrg {
        id: body["id"].as_str().unwrap().parse().unwrap(),
        slug,
    }
}
