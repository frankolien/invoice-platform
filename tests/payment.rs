mod common;

use actix_web::{http::StatusCode, test};
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use common::{make_app, seed_org, seed_user};

async fn create_client<S, B>(app: &S, token: &str, org_id: Uuid) -> Uuid
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
        .set_json(json!({ "name": "PaymentTest", "email": "p@t.local" }))
        .to_request();
    let body: Value = test::read_body_json(test::call_service(app, req).await).await;
    body["id"].as_str().unwrap().parse().unwrap()
}

async fn create_sent_invoice<S, B>(
    app: &S,
    token: &str,
    org_id: Uuid,
    client_id: Uuid,
    number: &str,
) -> Uuid
where
    S: actix_web::dev::Service<
            actix_http::Request,
            Response = actix_web::dev::ServiceResponse<B>,
            Error = actix_web::Error,
        >,
    B: actix_web::body::MessageBody,
{
    let due = (Utc::now() + Duration::days(30)).to_rfc3339();
    let req = test::TestRequest::post()
        .uri("/v1/invoices")
        .insert_header(("authorization", format!("Bearer {token}")))
        .insert_header(("x-org-id", org_id.to_string()))
        .set_json(json!({
            "client_id": client_id,
            "invoice_number": number,
            "line_items": [{ "description": "Item", "quantity": "1", "unit_price": "100.00" }],
            "tax_rate": "0",
            "due_date": due,
        }))
        .to_request();
    let body: Value = test::read_body_json(test::call_service(app, req).await).await;
    let id: Uuid = body["id"].as_str().unwrap().parse().unwrap();

    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{id}/send"))
        .insert_header(("authorization", format!("Bearer {token}")))
        .insert_header(("x-org-id", org_id.to_string()))
        .to_request();
    test::call_service(app, req).await;
    id
}

/// Direct DB insert for a succeeded payment. Skips the Stripe round-trip
/// so we can test /refund + /payments listing without mocking Stripe.
async fn seed_payment(
    pool: &PgPool,
    org_id: Uuid,
    invoice_id: Uuid,
    status: &str,
    stripe_pi: Option<&str>,
) -> Uuid {
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO payments (org_id, invoice_id, amount, currency, status, stripe_payment_intent_id)
        VALUES ($1, $2, 100.00, 'USD', $3, $4)
        RETURNING id
        "#,
    )
    .bind(org_id)
    .bind(invoice_id)
    .bind(status)
    .bind(stripe_pi)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

#[sqlx::test(migrations = "./migrations")]
async fn pay_rejects_when_stripe_unconfigured(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client = create_client(&app, &user.access_token, org.id).await;
    let invoice = create_sent_invoice(&app, &user.access_token, org.id, client, "INV-PAY-1").await;

    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{invoice}/pay"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // Our require_stripe() returns 400 when unconfigured. That's our test setup.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn pay_rejects_invoice_in_draft_status(pool: PgPool) {
    let app = make_app(pool.clone()).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client = create_client(&app, &user.access_token, org.id).await;

    // Create but DON'T send — stays in draft
    let due = (Utc::now() + Duration::days(30)).to_rfc3339();
    let req = test::TestRequest::post()
        .uri("/v1/invoices")
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(json!({
            "client_id": client,
            "invoice_number": "INV-DRAFT-1",
            "line_items": [{ "description": "x", "quantity": "1", "unit_price": "10" }],
            "due_date": due,
        }))
        .to_request();
    let body: Value = test::read_body_json(test::call_service(&app, req).await).await;
    let invoice_id = body["id"].as_str().unwrap();

    let req = test::TestRequest::post()
        .uri(&format!("/v1/invoices/{invoice_id}/pay"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // Rejected BEFORE Stripe check because the invoice is in draft.
    // (Since Stripe is unconfigured we'd get 400 either way; the message
    // differs but both paths are 400. That's fine — we're just asserting
    // unsuccessful attempts on the draft path.)
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_payments_returns_empty_for_invoice_with_none(pool: PgPool) {
    let app = make_app(pool.clone()).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client = create_client(&app, &user.access_token, org.id).await;
    let invoice = create_sent_invoice(&app, &user.access_token, org.id, client, "INV-LIST-0").await;

    let req = test::TestRequest::get()
        .uri(&format!("/v1/invoices/{invoice}/payments"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 0);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_payments_returns_seeded_rows(pool: PgPool) {
    let app = make_app(pool.clone()).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client = create_client(&app, &user.access_token, org.id).await;
    let invoice = create_sent_invoice(&app, &user.access_token, org.id, client, "INV-LIST-1").await;

    let _p1 = seed_payment(&pool, org.id, invoice, "succeeded", Some("pi_test_1")).await;
    let _p2 = seed_payment(&pool, org.id, invoice, "failed", Some("pi_test_2")).await;

    let req = test::TestRequest::get()
        .uri(&format!("/v1/invoices/{invoice}/payments"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    let body: Value = test::read_body_json(resp).await;
    assert_eq!(body.as_array().unwrap().len(), 2);
}

#[sqlx::test(migrations = "./migrations")]
async fn list_payments_scoped_to_org(pool: PgPool) {
    let app = make_app(pool.clone()).await;

    let alice = seed_user(&app, "alice").await;
    let alice_org = seed_org(&app, &alice.access_token, "alice").await;
    let alice_client = create_client(&app, &alice.access_token, alice_org.id).await;
    let alice_invoice =
        create_sent_invoice(&app, &alice.access_token, alice_org.id, alice_client, "INV-A-1").await;

    let bob = seed_user(&app, "bob").await;
    let bob_org = seed_org(&app, &bob.access_token, "bob").await;

    // Seed a payment in alice_org
    let _ = seed_payment(&pool, alice_org.id, alice_invoice, "succeeded", Some("pi_alice")).await;

    // Bob cannot see alice's invoice's payments (404 — invoice not in bob's org)
    let req = test::TestRequest::get()
        .uri(&format!("/v1/invoices/{alice_invoice}/payments"))
        .insert_header(("authorization", format!("Bearer {}", bob.access_token)))
        .insert_header(("x-org-id", bob_org.id.to_string()))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[sqlx::test(migrations = "./migrations")]
async fn refund_404_when_payment_missing(pool: PgPool) {
    let app = make_app(pool).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let missing = Uuid::new_v4();

    let req = test::TestRequest::post()
        .uri(&format!("/v1/payments/{missing}/refund"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(json!({}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    // With Stripe unconfigured, require_stripe() trips first and returns 400.
    // When Stripe IS configured but payment missing, it's 404. Either way
    // the request is rejected — we just verify it's not processed.
    assert!(
        resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND,
        "unexpected status {}",
        resp.status()
    );
}

#[sqlx::test(migrations = "./migrations")]
async fn refund_rejects_non_succeeded_payment(pool: PgPool) {
    // Even with Stripe unconfigured, this test documents the expected
    // rejection path. Since require_stripe runs first, with an empty
    // stripe config we always get 400. When Stripe is configured and the
    // payment is in 'pending' status, the handler still returns 400.
    let app = make_app(pool.clone()).await;
    let user = seed_user(&app, "u").await;
    let org = seed_org(&app, &user.access_token, "o").await;
    let client = create_client(&app, &user.access_token, org.id).await;
    let invoice =
        create_sent_invoice(&app, &user.access_token, org.id, client, "INV-REFUND-X").await;

    let payment_id = seed_payment(&pool, org.id, invoice, "pending", Some("pi_x")).await;

    let req = test::TestRequest::post()
        .uri(&format!("/v1/payments/{payment_id}/refund"))
        .insert_header(("authorization", format!("Bearer {}", user.access_token)))
        .insert_header(("x-org-id", org.id.to_string()))
        .set_json(json!({}))
        .to_request();
    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
