mod common;

use actix_web::{http::StatusCode, test};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::{make_app, seed_org, seed_user};

#[sqlx::test(migrations = "./migrations")]
async fn create_and_get_org(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "owner").await;
    let org = seed_org(&app, &user.access_token, "acme").await;

    let req = test::TestRequest::get()
        .uri(&format!("/v1/organizations/{}", org.id))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["slug"], org.slug);
    assert_eq!(body["plan"], "free");
}

#[sqlx::test(migrations = "./migrations")]
async fn non_member_cannot_read_org(pool: PgPool) {
    let app = make_app(pool).await;
    let owner = seed_user(&app, "owner").await;
    let org = seed_org(&app, &owner.access_token, "private").await;

    let outsider = seed_user(&app, "outsider").await;
    let req = test::TestRequest::get()
        .uri(&format!("/v1/organizations/{}", org.id))
        .insert_header(("authorization", format!("Bearer {}", outsider.access_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[sqlx::test(migrations = "./migrations")]
async fn update_org_as_owner(pool: PgPool) {
    let app = make_app(pool).await;
    let owner = seed_user(&app, "owner").await;
    let org = seed_org(&app, &owner.access_token, "upd").await;

    let req = test::TestRequest::patch()
        .uri(&format!("/v1/organizations/{}", org.id))
        .insert_header(("authorization", format!("Bearer {}", owner.access_token)))
        .set_json(json!({ "name": "New Name", "plan": "pro" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["name"], "New Name");
    assert_eq!(body["plan"], "pro");
}

#[sqlx::test(migrations = "./migrations")]
async fn invite_gives_access(pool: PgPool) {
    let app = make_app(pool).await;
    let owner = seed_user(&app, "owner").await;
    let org = seed_org(&app, &owner.access_token, "team").await;
    let invitee = seed_user(&app, "invitee").await;

    // invite
    let req = test::TestRequest::post()
        .uri(&format!("/v1/organizations/{}/invite", org.id))
        .insert_header(("authorization", format!("Bearer {}", owner.access_token)))
        .set_json(json!({ "email": invitee.email, "role": "admin" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // invitee can now read
    let req = test::TestRequest::get()
        .uri(&format!("/v1/organizations/{}", org.id))
        .insert_header(("authorization", format!("Bearer {}", invitee.access_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_slug_conflicts(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "owner").await;

    let _ = seed_org(&app, &user.access_token, "same").await;

    // try creating again with identical slug by skipping the helper (since
    // seed_org generates a random suffix)
    let req = test::TestRequest::post()
        .uri("/v1/organizations")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .set_json(json!({ "name": "same", "slug": "fixed-slug" }))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), StatusCode::CREATED);

    let req = test::TestRequest::post()
        .uri("/v1/organizations")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .set_json(json!({ "name": "same2", "slug": "fixed-slug" }))
        .to_request();
    assert_eq!(test::call_service(&app, req).await.status(), StatusCode::CONFLICT);
}

#[sqlx::test(migrations = "./migrations")]
async fn missing_auth_is_unauthorized(pool: PgPool) {
    let app = make_app(pool).await;
    let req = test::TestRequest::post()
        .uri("/v1/organizations")
        .set_json(json!({ "name": "x", "slug": "no-auth" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}
