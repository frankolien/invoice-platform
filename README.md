# Invoice Platform (Rust / Actix Web)

A port of the Node/Fastify Invoice Platform to Rust, using Actix Web and Postgres.

This is an **MVP skeleton** — the goal of this iteration is a compiling, runnable backend with the core resources (auth, orgs, clients, invoices). The rest (payments, queues, webhooks, recurring invoices, analytics, observability stack) will be layered on incrementally.

## Stack

- **Web:** Actix Web 4
- **Database:** Postgres via sqlx (async, compile-time-free runtime queries)
- **Auth:** JWT (HS256) + Argon2 password hashing
- **Logging:** tracing + tracing-subscriber (JSON output)
- **Metrics:** prometheus crate at `/metrics`
- **Decimals:** rust_decimal (no float weirdness for money)

## Running

### With Docker Compose (recommended)

```bash
docker compose up --build
```

Brings up Postgres and the API. Migrations run automatically on API boot.

### Locally

1. Start Postgres:
   ```bash
   docker run --rm -p 5432:5432 \
     -e POSTGRES_USER=invoice -e POSTGRES_PASSWORD=invoice -e POSTGRES_DB=invoice_platform \
     postgres:16-alpine
   ```
2. Copy env: `cp .env.example .env`
3. Run: `cargo run`

## Endpoints

### Public
- `GET /` — service info
- `GET /health` — liveness + Postgres check
- `GET /ready` — readiness
- `GET /metrics` — Prometheus metrics

### `/v1/auth`
- `POST /register` — `{ email, password, name }`
- `POST /login` — `{ email, password }`
- `POST /refresh` — `{ refresh_token }`

### `/v1/organizations` (Bearer token required)
- `POST /` — create org, creator becomes owner
- `GET /:id`
- `PATCH /:id` — owner/admin only
- `POST /:id/invite` — add member by email

### `/v1/clients` (Bearer + `x-org-id` header)
- `POST /`, `GET /`, `GET /:id`, `PATCH /:id`, `DELETE /:id` (soft delete)

### `/v1/invoices` (Bearer + `x-org-id`)
- `POST /`, `GET /`, `GET /:id`, `PATCH /:id` (draft only)
- `POST /:id/send` — draft → sent
- `POST /:id/cancel`
- `POST /:id/viewed`

## Roadmap (not yet in this skeleton)

- Payments (Stripe) + idempotency
- Background jobs (apalis + Redis): invoice email, PDF gen, recurring invoices, overdue scan
- Outbound webhooks with HMAC signing
- Analytics endpoints
- Circuit breakers for external services
- OpenTelemetry tracing export
- Rate limiting
- Full tests

## Project layout

```
src/
├── main.rs                       # app bootstrap + graceful shutdown
├── config/                       # env loading
├── db/                           # sqlx pool + migrations
├── error/                        # AppError + ResponseError impl
├── auth/                         # jwt + password hashing
├── middleware/
│   ├── auth_user.rs              # Bearer-token extractor (FromRequest)
│   └── tenant.rs                 # x-org-id + membership check extractor
├── modules/
│   ├── auth/                     # register, login, refresh
│   ├── organization/             # create, get, update, invite
│   ├── client/                   # crud + soft delete
│   └── invoice/                  # crud + send/cancel/viewed
└── observability/                # tracing init, health, metrics
migrations/                       # sqlx-managed SQL
```
