mod common;

use actix_web::{http::StatusCode, test};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::{make_app, seed_org, seed_user};

#[sqlx::test(migrations = "./migrations")]
async fn user_cannot_see_another_orgs_clients(pool: PgPool) {
    let app = make_app(pool).await;

    let alice = seed_user(&app, "alice").await;
    let alice_org = seed_org(&app, &alice.access_token, "alice-org").await;

    let bob = seed_user(&app, "bob").await;
    let bob_org = seed_org(&app, &bob.access_token, "bob-org").await;

    // Bob creates a client in his org
    let req = test::TestRequest::post()
        .uri("/v1/clients")
        .insert_header(("authorization", format!("Bearer {}", bob.access_token)))
        .insert_header(("x-org-id", bob_org.id.to_string()))
        .set_json(json!({ "name": "Bob's client", "email": "c@bob.test" }))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    let bob_client_id = body["id"].as_str().unwrap().to_string();

    // Alice, authenticated, with HER org id: cannot see Bob's client via get
    let req = test::TestRequest::get()
        .uri(&format!("/v1/clients/{bob_client_id}"))
        .insert_header(("authorization", format!("Bearer {}", alice.access_token)))
        .insert_header(("x-org-id", alice_org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Alice, authenticated, trying to forge as Bob's org: forbidden (not a member)
    let req = test::TestRequest::get()
        .uri(&format!("/v1/clients/{bob_client_id}"))
        .insert_header(("authorization", format!("Bearer {}", alice.access_token)))
        .insert_header(("x-org-id", bob_org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Alice's client list in her org: empty
    let req = test::TestRequest::get()
        .uri("/v1/clients")
        .insert_header(("authorization", format!("Bearer {}", alice.access_token)))
        .insert_header(("x-org-id", alice_org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["total"], 0);
}
