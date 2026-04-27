# Invoice Platform (Rust / Actix Web)

A multi-tenant invoicing API ported from the Node/Fastify [Invoice Platform](../Invoice-platform/). Stripe payments, recurring invoices, outbound webhooks, background workers, rate limiting, circuit breakers, Prometheus metrics, audit log.

## Stack

| Layer | Choice |
|---|---|
| Web | Actix Web 4 |
| Database | Postgres via sqlx (async, runtime queries) |
| Cache + queue | Redis (via apalis 0.7) |
| Auth | JWT (HS256) + Argon2 |
| Payments | async-stripe 0.40 |
| Webhooks (out) | reqwest, HMAC-SHA256 signed |
| Background jobs | apalis + apalis-cron |
| Reliability | Custom circuit breaker (3 instances) |
| Logging | tracing + tracing-subscriber (JSON) |
| Metrics | prometheus crate at `/metrics` |
| Decimals | rust_decimal (no float weirdness for money) |

## Running

### With Docker Compose

```bash
docker compose up -d            # full stack: api + postgres + redis
docker compose up -d postgres redis   # just deps (for cargo run / cargo test)
```

Postgres is mapped to host **port 5433** to avoid colliding with a local Postgres on 5432.
Redis is on **port 6380** for the same reason.

### Locally

```bash
docker compose up -d postgres redis
cp .env.example .env       # edit secrets if you want
cargo run
```

### Tests

```bash
DATABASE_URL="postgres://invoice:invoice@localhost:5433/invoice_platform" \
REDIS_URL="redis://localhost:6380" \
cargo test
```

28 tests: 2 unit (circuit breaker) + 5 auth + 6 org + 3 client + 4 invoice + 1 isolation + 7 payment.

## Endpoints

### Public
- `GET /` — service info
- `GET /health`, `GET /ready` — probes
- `GET /metrics` — Prometheus

### `/v1/auth`
- `POST /register`, `POST /login`, `POST /refresh`

### `/v1/organizations` (Bearer)
- `POST /`, `GET /:id`, `PATCH /:id`, `POST /:id/invite`

### `/v1/clients` (Bearer + `x-org-id`)
- Full CRUD + soft delete

### `/v1/invoices` (Bearer + `x-org-id`)
- `POST /`, `GET /`, `GET /:id`, `PATCH /:id` (draft only)
- `POST /:id/send`, `POST /:id/cancel`, `POST /:id/viewed`
- `POST /:id/pay` — Stripe checkout, supports `Idempotency-Key`
- `GET /:id/payments` — list payments

### `/v1/payments` (Bearer + `x-org-id`)
- `POST /:id/refund` — full or partial

### `/v1/recurring-invoices` (Bearer + `x-org-id`)
- Full CRUD + `pause` / `resume` / `cancel`

### `/v1/webhook-subscriptions` (Bearer + `x-org-id`)
- Full CRUD + `POST /:id/test` (synchronous test delivery)

### `/v1/analytics` (Bearer + `x-org-id`)
- `GET /revenue?from=&to=&currency=` — total + monthly breakdown
- `GET /invoices?from=&to=` — counts per status + live overdue summary

### `/v1/webhooks/stripe`
- Stripe webhook receiver (signature verified, deduplicated via Redis 48h)

## Background workers

All run in-process as tokio tasks, spawned at boot.

| Worker | Trigger | Concurrency | Retries |
|---|---|---|---|
| `invoice-email` | After `POST /invoices/:id/send` | 5 | 3 |
| `webhook-delivery` | Business event → `dispatch_webhooks` | 10 | 8 |
| `recurring-create` | Scanned by cron, materializes a real invoice | 3 | 3 |
| `overdue-scan` (cron) | Hourly | 1 | — |
| `recurring-scan` (cron) | Every 15 minutes | 1 | — |

Email is currently a `LogEmailSender` stub. Implement the `EmailSender` trait for Resend/SES/SMTP and swap in [src/jobs/mod.rs](src/jobs/mod.rs).

## Outbound webhook events

Emitted automatically when the corresponding state change occurs:

- `invoice.created`, `invoice.sent`, `invoice.paid`, `invoice.overdue`, `invoice.cancelled`
- `payment.succeeded`, `payment.failed`

Customers register via `POST /v1/webhook-subscriptions` and receive an HMAC-signed POST. Headers: `X-Signature`, `X-Timestamp`, `X-Event-Type`, `X-Delivery-Id`.

## Reliability

### Circuit breakers ([src/circuit_breaker/mod.rs](src/circuit_breaker/mod.rs))

Three pre-configured instances, mirroring the TS app:

| Service | Failure threshold | Cooldown | Recovery threshold |
|---|---|---|---|
| Stripe | 5 | 30s | 2 |
| Email | 3 | 60s | 1 |
| Webhook | 10 | 15s | 3 |

State exported as `circuit_breaker_state{service}` Prometheus gauge.

### Idempotency

`POST /v1/invoices/:id/pay` honors an `Idempotency-Key` header. Same key within 24h returns the cached response without re-creating a Stripe session.

### Stripe webhook deduplication

Each Stripe event ID stored in Redis with 48h TTL. Replays return `200 {deduplicated: true}` without re-processing.

### Rate limiting

200 req/min per `x-org-id` (or per IP if no org). Redis fixed-window. Returns `429` with `Retry-After`. Skips `/health`, `/ready`, `/metrics`. Fails open if Redis is down.

### Audit log

Every POST/PATCH/PUT/DELETE under `/v1` logs `{user_id, org_id, method, path, status}` as a structured tracing event.

## Metrics

`GET /metrics` (Prometheus text format):

| Metric | Type | Labels |
|---|---|---|
| `http_requests_total` | counter | method, route, status |
| `http_request_duration_seconds` | histogram | method, route, status |
| `invoices_created_total` | counter | org_id |
| `payments_processed_total` | counter | org_id, status |
| `circuit_breaker_state` | gauge | service (0=closed, 1=open, 2=half-open) |

Route labels use Actix's matched pattern (e.g. `/v1/invoices/{id}`) so cardinality stays bounded.

## Project layout

```
src/
├── main.rs                       # bootstrap, worker spawn, graceful shutdown
├── lib.rs                        # AppState + build_app factory
├── config/                       # env loading
├── db/                           # sqlx pool + migrations
├── cache/                        # Redis wrapper
├── circuit_breaker/              # CLOSED/OPEN/HALF_OPEN state machine
├── error/                        # AppError + ResponseError impl
├── auth/                         # JWT + Argon2
├── middleware/
│   ├── auth_user.rs              # Bearer JWT extractor
│   ├── tenant.rs                 # x-org-id + membership extractor
│   ├── rate_limit.rs             # Redis fixed-window
│   ├── audit_log.rs              # structured audit events
│   └── metrics.rs                # HTTP request counter + histogram
├── modules/
│   ├── auth/                     # register/login/refresh
│   ├── organization/             # create/get/update/invite
│   ├── client/                   # CRUD + soft delete
│   ├── invoice/                  # CRUD + status transitions + service fn
│   ├── payment/                  # Stripe checkout + refund + webhook
│   ├── recurring_invoice/        # CRUD + pause/resume/cancel
│   ├── webhook_subscription/     # CRUD + /test
│   └── analytics/                # revenue + invoices reports
├── jobs/
│   ├── mod.rs                    # JobQueues + spawn_workers + dispatch_webhooks
│   ├── email.rs                  # EmailSender trait + LogEmailSender stub
│   ├── invoice_email.rs          # SendInvoiceEmail handler
│   ├── webhook_delivery.rs       # HMAC-signed delivery + reqwest
│   ├── recurring.rs              # scan + materialize
│   └── overdue.rs                # cron handler
└── observability/
    ├── mod.rs                    # tracing init
    ├── health.rs                 # / + /health + /ready
    └── metrics.rs                # Prometheus registry + /metrics
migrations/                       # sqlx-managed SQL
tests/                            # integration tests
```

## Load testing

Four [k6](https://k6.io) scripts in [k6/](k6/), adapted from the TS app's scripts to match this API's payload shape (`snake_case`, decimal-as-string, page/page_size pagination).

| Script | What it tests | VU ramp | Thresholds |
|---|---|---|---|
| `auth-load.js` | register → login → refresh | 0 → 100 over 2.5m | P95 login < 300ms, P95 register < 500ms, errors < 5% |
| `invoice-load.js` | create + list + paginate + get | 0 → 200 over 3m | P95 create < 800ms, P95 list < 500ms, P95 get < 300ms |
| `payment-stress.js` | create+send+pay with idempotency-key | 0 → 50 over 2.5m | P95 payment < 1500ms, errors < 10% |
| `mixed-scenario.js` | 60% reads / 30% writes / 10% payments concurrently | 100 VUs for 3m | P95 read < 500ms, P95 write < 1s, P95 payment < 2s |

```bash
brew install k6                                          # macOS
k6 run k6/auth-load.js                                   # default target
k6 run --env BASE_URL=http://staging:3000 k6/auth-load.js
```

Note: `payment-stress.js` and the payment leg of `mixed-scenario.js` accept either `201` (Stripe configured + checkout session created) or `400` ("stripe is not configured") as success — so they're useful in dev without real Stripe keys.

## What's not (yet) ported from the Node original

- OpenTelemetry tracing export (Jaeger) — wired but disabled, SDK 0.31 lifecycle issue under investigation
- PDF generation
- OpenAPI / Swagger UI
- Bull Board queue dashboard equivalent
- Real email sender (Resend/SES) — only the log-only stub
