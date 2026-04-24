mod common;

use actix_web::{http::StatusCode, test};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;

use common::{make_app, seed_org, seed_user};

async fn create_client<S, B>(
    app: &S,
    token: &str,
    org_id: &uuid::Uuid,
) -> uuid::Uuid
where
    S: actix_web::dev::Service<
            actix_http::Request,
            Response = actix_web::dev::ServiceResponse<B>,
            Error = actix_web::Error,
        >,
    B: actix_web::body::MessageBody,
{
    let req = test::TestRequest::post()
        .uri("/v1/clients")
        .insert_header(("authorization", format!("Bearer {token}")))
        .insert_header(("x-org-id", org_id.to_string()))
        .set_json(json!({
            "name": "Test Client",
            "email": "client@test.local",
        }))
        .to_request();
    let resp = test::call_service(app, req).await;
    let body: Value = test::read_body_json(resp).await;
    body["id"].as_str().unwrap().parse().unwrap()
}

fn invoice_payload(client_id: uuid::Uuid, number: &str) -> Value {
    let due = (Utc::now() + Duration::days(30)).to_rfc3339();
    json!({
        "client_id": client_id,
        "invoice_number": number,
        "line_items": [
            { "description": "Widget", "quantity": "2", "unit_price": "50.00" },
            { "description": "Gadget", "quantity": "1", "unit_price": "25.00" },
        ],
        "tax_rate": "0.10",
        "currency": "USD",
        "due_date": due,
    })
}

#[sqlx::test(migrations = "./migrations")]
async fn create_computes_totals(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client_id = create_client(&app, &user.access_token, &org.id).await;

    let req = test::TestRequest::post()
        .uri("/v1/invoices")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(invoice_payload(client_id, "INV-001"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body: Value = test::read_body_json(resp).await;
    // subtotal = 100 + 25 = 125 ; tax = 12.5 ; total = 137.5
    assert_eq!(body["subtotal"], "125.00");
    assert_eq!(body["tax_amount"], "12.50");
    assert_eq!(body["total"], "137.50");
    assert_eq!(body["status"], "draft");
}

#[sqlx::test(migrations = "./migrations")]
async fn duplicate_invoice_number_in_org_conflicts(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client_id = create_client(&app, &user.access_token, &org.id).await;

    let make_req = || {
        test::TestRequest::post()
            .uri("/v1/invoices")
            .insert_header(("authorization", format!("Bearer {}", user.access_token)))
            .insert_header(("x-org-id", org.id.to_string()))
            .set_json(invoice_payload(client_id, "INV-DUPE"))
            .to_request()
    };

    assert_eq!(
        test::call_service(&app, make_req()).await.status(),
        StatusCode::CREATED
    );
    assert_eq!(
        test::call_service(&app, make_req()).await.status(),
        StatusCode::CONFLICT
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn status_transitions(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client_id = create_client(&app, &user.access_token, &org.id).await;

    let req = test::TestRequest::post()
        .uri("/v1/invoices")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(invoice_payload(client_id, "INV-STATE"))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    let id = body["id"].as_str().unwrap().to_string();

    // send: draft -> sent
    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{id}/send"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "sent");
    assert!(body["sent_at"].is_string());

    // send again: now it's not draft -> 400
    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{id}/send"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    assert_eq!(
        test::call_service(&app, req).await.status(),
        StatusCode::BAD_REQUEST
    );

    // viewed: sent -> viewed
    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{id}/viewed"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "viewed");

    // cancel
    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{id}/cancel"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body["status"], "cancelled");
}

#[sqlx::test(migrations = "./migrations")]
async fn update_rejected_on_non_draft(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client_id = create_client(&app, &user.access_token, &org.id).await;

    // create + send
    let req = test::TestRequest::post()
        .uri("/v1/invoices")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(invoice_payload(client_id, "INV-LOCKED"))
        .to_request();
    let body: Value = test::read_body_json(test::call_service(&app, req).await).await;
    let id = body["id"].as_str().unwrap().to_string();

    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{id}/send"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    test::call_service(&app, req).await;

    // try to edit a sent invoice
    let req = test::TestRequest::patch()
        .uri(&format!("/v1/invoices/{id}"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(json!({ "notes": "late edit" }))
        .to_request();
    assert_eq!(
        test::call_service(&app, req).await.status(),
        StatusCode::BAD_REQUEST
    );
}
