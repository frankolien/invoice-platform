mod common;

use actix_web::{http::StatusCode, test};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::make_app;

#[sqlx::test(migrations = "./migrations")]
async fn register_login_refresh_flow(pool: PgPool) {
    let app = make_app(pool).await;

    // register
    let req = test::TestRequest::post()
        .uri("/v1/auth/register")
        .set_json(json!({
            "email": "flow@test.local",
            "password": "password123",
            "name": "Flow User",
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(resp).await;
    assert!(body["access_token"].is_string());
    assert!(body["refresh_token"].is_string());
    assert_eq!(body["user"]["email"], "flow@test.local");

    // login
    let req = test::TestRequest::post()
        .uri("/v1/auth/login")
        .set_json(json!({
            "email": "flow@test.local",
            "password": "password123",
        }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let login_body: Value = test::read_body_json(resp).await;
    let refresh_token = login_body["refresh_token"].as_str().unwrap();

    // refresh
    let req = test::TestRequest::post()
        .uri("/v1/auth/refresh")
        .set_json(json!({ "refresh_token": refresh_token }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let refresh_body: Value = test::read_body_json(resp).await;
    assert!(refresh_body["access_token"].is_string());
}

#[sqlx::test(migrations = "./migrations")]
async fn register_rejects_duplicate_email(pool: PgPool) {
    let app = make_app(pool).await;

    let payload = json!({
        "email": "dupe@test.local",
        "password": "password123",
        "name": "Dupe",
    });

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/auth/register")
            .set_json(&payload)
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CREATED);

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/auth/register")
            .set_json(&payload)
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn login_rejects_wrong_password(pool: PgPool) {
    let app = make_app(pool).await;

    test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/auth/register")
            .set_json(json!({
                "email": "wrongpw@test.local",
                "password": "password123",
                "name": "X",
            }))
            .to_request(),
    )
    .await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/auth/login")
            .set_json(json!({
                "email": "wrongpw@test.local",
                "password": "nope-wrong",
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrations = "./migrations")]
async fn register_rejects_short_password(pool: PgPool) {
    let app = make_app(pool).await;

    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/auth/register")
            .set_json(json!({
                "email": "short@test.local",
                "password": "short",
                "name": "X",
            }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn refresh_rejects_access_token(pool: PgPool) {
    let app = make_app(pool).await;

    let user = common::seed_user(&app, "typeguard").await;

    // access token should NOT be accepted by /refresh
    let resp = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/v1/auth/refresh")
            .set_json(json!({ "refresh_token": user.access_token }))
            .to_request(),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
