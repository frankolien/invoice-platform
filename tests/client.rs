mod common;

use actix_web::{http::StatusCode, test};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::{make_app, seed_org, seed_user};

fn auth_and_org(token: &str, org_id: &uuid::Uuid) -> [(&'static str, String); 2] {
    [
        ("authorization", format!("Bearer {token}")),
        ("x-org-id", org_id.to_string()),
    ]
}

#[sqlx::test(migrations = "./migrations")]
async fn client_crud_and_soft_delete(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let headers = auth_and_org(&user.access_token, &org.id);

    // create
    let mut req = test::TestRequest::post()
        .uri("/v1/clients")
        .set_json(json!({
            "name": "Acme Corp",
            "email": "billing@acme.test",
            "phone": "555-1234",
        }));
    for h in &headers {
        req = req.insert_header((h.0, h.1.clone()));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(resp).await;
    let client_id = body["id"].as_str().unwrap().to_string();
    assert_eq!(body["name"], "Acme Corp");

    // list contains it
    let mut req = test::TestRequest::get().uri("/v1/clients");
    for h in &headers {
        req = req.insert_header((h.0, h.1.clone()));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["total"], 1);

    // update
    let mut req = test::TestRequest::patch()
        .uri(&format!("/v1/clients/{client_id}"))
        .set_json(json!({ "phone": "555-9999" }));
    for h in &headers {
        req = req.insert_header((h.0, h.1.clone()));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["phone"], "555-9999");
    assert_eq!(body["name"], "Acme Corp"); // unchanged

    // soft delete
    let mut req = test::TestRequest::delete().uri(&format!("/v1/clients/{client_id}"));
    for h in &headers {
        req = req.insert_header((h.0, h.1.clone()));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    // get now 404s
    let mut req = test::TestRequest::get().uri(&format!("/v1/clients/{client_id}"));
    for h in &headers {
        req = req.insert_header((h.0, h.1.clone()));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn missing_org_header_is_bad_request(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;

    let req = test::TestRequest::get()
        .uri("/v1/clients")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_supports_search(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let headers = auth_and_org(&user.access_token, &org.id);

    for name in ["Apple Inc", "Banana Bros", "Apricot LLC"] {
        let mut req = test::TestRequest::post()
            .uri("/v1/clients")
            .set_json(json!({
                "name": name,
                "email": format!("{}@x.test", name.replace(' ', "").to_lowercase()),
            }));
        for h in &headers {
            req = req.insert_header((h.0, h.1.clone()));
        }
        test::call_service(&app, req.to_request()).await;
    }

    let mut req = test::TestRequest::get().uri("/v1/clients?search=Ap");
    for h in &headers {
        req = req.insert_header((h.0, h.1.clone()));
    }
    let resp = test::call_service(&app, req.to_request()).await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["total"], 2); // Apple + Apricot
}
