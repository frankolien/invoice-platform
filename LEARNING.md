# The Qent Backend: A Complete Rust + Actix-Web Guide

**From Zero Rust to Full Understanding of This Codebase**

Written for a developer who knows JavaScript/Dart but is learning Rust by understanding every line of the Qent car rental platform backend.

---

## Table of Contents

- [Part 1: Rust Fundamentals You Need](#part-1-rust-fundamentals-you-need)
- [Part 2: Project Structure](#part-2-project-structure)
- [Part 3: Actix-Web Framework Deep Dive](#part-3-actix-web-framework-deep-dive)
- [Part 4: Database with SQLx](#part-4-database-with-sqlx)
- [Part 5: Authentication & Security](#part-5-authentication--security)
- [Part 6: Every Handler Explained](#part-6-every-handler-explained)
- [Part 7: WebSocket Implementation](#part-7-websocket-implementation)
- [Part 8: Payment Integration (Paystack)](#part-8-payment-integration-paystack)
- [Part 9: Models & Data Layer](#part-9-models--data-layer)
- [Part 10: Error Handling Patterns](#part-10-error-handling-patterns)
- [Part 11: Testing](#part-11-testing)
- [Part 12: Deployment & Configuration](#part-12-deployment--configuration)
- [Part 13: Common Patterns Reference](#part-13-common-patterns-reference)

---

# Part 1: Rust Fundamentals You Need

Before diving into the codebase, you need to understand the Rust concepts that appear on virtually every line. If you know JavaScript or Dart, Rust will feel alien at first -- but once you understand *why* it works this way, it becomes incredibly powerful.

## 1.1 Ownership: The Big Idea

In JavaScript, you can do this freely:

```javascript
let name = "Emeka";
let greeting = name;       // Both variables point to the same string
console.log(name);         // Still works fine
```

In Rust, values have exactly ONE owner at a time:

```rust
let name = String::from("Emeka");
let greeting = name;       // Ownership MOVED to `greeting`
// println!("{}", name);   // ERROR! `name` no longer owns the data
println!("{}", greeting);  // This works
```

**Why does Rust do this?** Memory safety without garbage collection. In JavaScript, a garbage collector runs periodically to clean up unused memory. In Dart, same thing. Rust instead tracks ownership at compile time -- the compiler knows exactly when to free memory because there is always exactly one owner, and when that owner goes out of scope, the memory is freed.

**Where you see this in Qent:**

In `src/main.rs`, line 124:
```rust
let bg_pool = pool.clone();
tokio::spawn(auto_complete_bookings(bg_pool));
```

We need to `clone()` the database pool because `tokio::spawn` takes ownership of what you pass into it. If we passed `pool` directly, we could not use `pool` again on line 168 inside the `HttpServer::new` closure. The `.clone()` creates a new owned copy. For `PgPool`, this is cheap -- it is an `Arc` (reference-counted pointer) internally, so cloning just increments a counter.

## 1.2 Borrowing: Lending Without Giving Away

Instead of moving ownership, you can *borrow* a value with `&`:

```rust
fn greet(name: &str) {     // Borrows name, doesn't take ownership
    println!("Hello, {}", name);
}

let name = String::from("Fatima");
greet(&name);               // Lend name to greet()
println!("{}", name);       // Still works! We only lent it
```

**Mutable borrowing** (`&mut`) lets you modify borrowed data, but only one mutable borrow can exist at a time:

```rust
fn add_suffix(name: &mut String) {
    name.push_str(" (Host)");
}
```

**Where you see this in Qent:**

In `src/handlers/auth.rs`, line 46:
```rust
let password_hash = match hash(&body.password, DEFAULT_COST) {
```
The `&body.password` borrows the password string. The `hash` function only needs to read it, not own it.

In `src/middleware/auth.rs`, line 33:
```rust
req.extensions_mut().insert(claims);
```
`extensions_mut()` returns a mutable borrow of the request's extensions, letting us insert data.

## 1.3 Lifetimes: How Long a Borrow Lives

Lifetimes ensure that borrowed references don't outlive the data they point to. In most of the Qent codebase, Rust infers lifetimes automatically (called "lifetime elision"), so you rarely see explicit lifetime annotations like `'a`.

The key rule: a reference cannot outlive what it borrows.

```rust
// This would NOT compile:
fn bad_function() -> &str {
    let s = String::from("hello");
    &s  // ERROR: `s` is dropped when function ends,
        // but we're trying to return a reference to it
}

// This works -- the string literal lives for the entire program:
fn good_function() -> &'static str {
    "hello"  // String literals have 'static lifetime
}
```

**Where this matters in Qent:**

Most handler functions take ownership of data or use extractors that manage lifetimes for you. The `web::Json<T>` extractor, for example, owns the parsed JSON data -- you do not deal with lifetime annotations at all.

## 1.4 Result<T, E> and Option<T>: No null, No Exceptions

This is perhaps the single most important concept for understanding the Qent codebase. Rust has NO null and NO exceptions. Instead:

- `Option<T>` represents a value that might not exist: `Some(value)` or `None`
- `Result<T, E>` represents an operation that might fail: `Ok(value)` or `Err(error)`

**JavaScript comparison:**
```javascript
// JavaScript: function might return null/undefined or throw
function findUser(id) {
    const user = db.find(id);  // might be null
    if (!user) throw new Error("Not found");
    return user;
}
```

**Rust equivalent:**
```rust
// Rust: the return type tells you it might fail
async fn find_user(id: Uuid) -> Result<Option<User>, sqlx::Error> {
    // Returns Ok(Some(user)) if found
    // Returns Ok(None) if not found
    // Returns Err(e) if database error
}
```

**Where you see Option<T> in Qent models** (`src/models/car.rs`):
```rust
pub struct Car {
    pub latitude: Option<f64>,     // Car might not have coordinates
    pub longitude: Option<f64>,
    pub rating: Option<f64>,       // Might not have any reviews yet
    pub trip_count: Option<i64>,
    pub available_from: Option<NaiveDate>,  // Might not have date limits
    pub host_name: Option<String>,
}
```

These `Option` fields directly mirror NULLABLE columns in your database. When a column can be NULL, the Rust type is `Option<T>`. When it's NOT NULL, it's just `T`.

**Where you see Result<T, E> everywhere** -- every database query returns a Result:
```rust
// From src/handlers/auth.rs, line 35-40:
let existing = sqlx::query_scalar::<_, bool>(
    "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)"
)
.bind(&body.email)
.fetch_one(pool.get_ref())
.await;    // This returns Result<bool, sqlx::Error>
```

## 1.5 Pattern Matching with match

`match` is Rust's version of a super-powered switch statement. Unlike JavaScript's `switch`, Rust's `match` is *exhaustive* -- you must handle every possible case.

This is the MOST common pattern in the Qent codebase:

```rust
// From src/handlers/auth.rs, lines 128-138:
let user = match user {
    Ok(Some(u)) => u,           // Query succeeded AND found a user
    Ok(None) => {               // Query succeeded but no user found
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "Invalid credentials"}))
    }
    Err(_) => {                 // Query itself failed (database error)
        return HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": "Internal server error"}))
    }
};
```

Let me break this down piece by piece:

1. `sqlx::query_as::<_, User>(...).fetch_optional(...)` returns `Result<Option<User>, sqlx::Error>`
2. `Ok(Some(u))` -- the database query succeeded AND returned a row. `u` is the User.
3. `Ok(None)` -- the database query succeeded but returned zero rows. No user with that email.
4. `Err(_)` -- the database query failed (connection error, SQL error, etc.). The `_` means we are ignoring the specific error.

This pattern replaces JavaScript's try/catch + null checks:

```javascript
// JavaScript equivalent:
try {
    const user = await db.query("SELECT * FROM users WHERE email = $1", [email]);
    if (!user) {
        return res.status(401).json({ error: "Invalid credentials" });
    }
    // use user...
} catch (err) {
    return res.status(500).json({ error: "Internal server error" });
}
```

## 1.6 The ? Operator: Early Return on Error

The `?` operator is syntactic sugar for: "If this Result is Err, return the error immediately. If it's Ok, unwrap the value."

```rust
// These two are equivalent:

// Without ?
let claims = match extract_claims(req, jwt_secret) {
    Ok(c) => c,
    Err(e) => return Err(e),
};

// With ?
let claims = extract_claims(req, jwt_secret)?;
```

**Where you see this in Qent:**

In `src/middleware/auth.rs`, line 32-33:
```rust
pub fn validate_token(req: &ServiceRequest, jwt_secret: &str) -> Result<(), Error> {
    let claims = extract_claims(req, jwt_secret)?;  // Returns error if token invalid
    req.extensions_mut().insert(claims);
    Ok(())
}
```

And in `src/main.rs`, line 97-98:
```rust
async fn auth_mw(req: ServiceRequest, next: Next<impl MessageBody>) -> Result<ServiceResponse<impl MessageBody>, Error> {
    validate_token(&req, &jwt_secret)?;  // If token invalid, request stops here
    next.call(req).await                 // If valid, continue to the handler
}
```

The `?` can only be used in functions that return `Result`. Most Qent handlers return `HttpResponse` directly (not Result), so they use `match` instead of `?`.

## 1.7 Traits: Rust's Version of Interfaces

Traits define shared behavior. They are similar to interfaces in Dart or TypeScript, but more powerful.

**Derive macros** automatically implement traits for your types. Here is what every derive in Qent models means:

```rust
// From src/models/user.rs, line 23:
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    // ...
}
```

| Trait | What It Does | JS/Dart Equivalent |
|-------|-------------|-------------------|
| `Debug` | Allows printing with `{:?}` format | `toString()` |
| `Clone` | Allows `.clone()` to make a copy | `{...obj}` spread operator |
| `Serialize` | Convert struct to JSON (via serde) | `JSON.stringify()` |
| `Deserialize` | Create struct from JSON (via serde) | `JSON.parse()` with type |
| `FromRow` | Create struct from database row (SQLx) | ORM model hydration |
| `PartialEq` | Allows `==` comparison | `===` operator |
| `Validate` | Enables validation rules | Joi/Yup validation |

**Serialize/Deserialize are the most important.** They are why you can write:
```rust
HttpResponse::Ok().json(user)  // Automatically converts User struct to JSON
```

And why this works:
```rust
body: web::Json<SignUpRequest>  // Automatically parses JSON body into SignUpRequest struct
```

Without `Serialize`, you could not convert to JSON. Without `Deserialize`, you could not parse from JSON. Without `FromRow`, SQLx could not map database rows to your structs.

## 1.8 Async/Await in Rust

If you know async/await from JavaScript or Dart, Rust's version will feel familiar -- but there are key differences.

**JavaScript:**
```javascript
async function getUser(id) {
    const user = await db.query("SELECT * FROM users WHERE id = $1", [id]);
    return user;
}
```

**Rust:**
```rust
async fn get_user(pool: &PgPool, id: Uuid) -> Result<Option<User>, sqlx::Error> {
    let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await;
    user
}
```

The key differences:

1. **Rust futures are lazy.** In JavaScript, calling an async function immediately starts execution. In Rust, calling an async function returns a Future that does nothing until you `.await` it.

2. **Rust needs a runtime.** JavaScript has a built-in event loop. Rust does not -- you need to use a runtime like `tokio`. That is why Qent's `main.rs` uses `#[actix_web::main]`:
   ```rust
   #[actix_web::main]   // This sets up the tokio async runtime
   async fn main() -> std::io::Result<()> {
   ```

3. **No implicit concurrency.** In JavaScript, multiple awaits in a loop run concurrently by default. In Rust, you must explicitly use `tokio::spawn` or `join!` for concurrency.

## 1.9 Closures: Anonymous Functions

Closures in Rust are like arrow functions in JavaScript, but with ownership rules.

```rust
// JavaScript:
const double = (x) => x * 2;

// Rust:
let double = |x| x * 2;
```

**Where closures appear in Qent:**

In `src/main.rs`, the entire app is built inside a closure:
```rust
HttpServer::new(move || {    // `move` captures variables by ownership
    let cors = Cors::default()
        // ...
    App::new()
        .wrap(cors)
        // ...
})
```

The `move` keyword means this closure takes ownership of `pool`, `config`, and `ws_manager` from the outer scope. Without `move`, the closure would try to borrow them, but the closure outlives the scope where they were created (it runs for each new HTTP connection), so it must own the data.

The route definitions also use closures:
```rust
.route("/signup", web::post().to(handlers::auth::sign_up))
```

Here, `handlers::auth::sign_up` is a function pointer (not a closure), but Actix-Web wraps it in a closure internally.

## 1.10 Enums: More Than Just Constants

Rust enums can hold data. They are closer to Dart's sealed classes or TypeScript's discriminated unions than to JavaScript's object-based enums.

```rust
// From src/models/booking.rs:
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "booking_status", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
pub enum BookingStatus {
    Pending,
    Approved,
    Rejected,
    Confirmed,
    Active,
    Completed,
    Cancelled,
}
```

This maps directly to a PostgreSQL ENUM type. The `#[sqlx(type_name = "booking_status")]` tells SQLx which Postgres enum this corresponds to. The `rename_all = "lowercase"` means `Pending` in Rust maps to `'pending'` in the database and JSON.

Enums with data (not used in Qent but important to understand):
```rust
enum Shape {
    Circle(f64),                    // Holds a radius
    Rectangle { width: f64, height: f64 },  // Holds named fields
    Point,                          // Holds no data
}
```

## 1.11 String Types: &str vs String

This trips up every new Rust developer. There are two main string types:

| Type | Analogy | Owned? | Growable? |
|------|---------|--------|-----------|
| `String` | Like JavaScript `String` | Yes | Yes |
| `&str` | Like a read-only view into a string | No (borrowed) | No |

```rust
let owned: String = String::from("Hello");   // Heap-allocated, you own it
let borrowed: &str = "Hello";                // Points to static data
let slice: &str = &owned;                    // Points into the owned String
```

**In the Qent codebase**, function parameters usually take `&str` (borrow) while struct fields use `String` (own):

```rust
// Struct owns its data:
pub struct User {
    pub email: String,       // User owns this string
    pub full_name: String,
}

// Function borrows:
pub fn hash(password: &str, cost: u32) -> Result<String, BcryptError> {
    // Reads the password, returns a new owned String
}
```

---

# Part 2: Project Structure

## 2.1 The Full Project Tree

```
qent/
|
|-- Cargo.toml                    # Project manifest (like package.json)
|-- Cargo.lock                    # Locked dependency versions (like package-lock.json)
|-- .env                          # Environment variables (DB URL, secrets)
|
|-- migrations/                   # SQL migration files (run in order)
|   |-- 001_initial.sql           # Core tables: users, cars, bookings, payments, reviews
|   |-- 002_additions_and_seed.sql # Favorites, notifications, mock data
|   |-- 003_saved_cards.sql       # Saved payment cards table
|   |-- 004_verification_codes.sql # Email verification codes
|   |-- 005_more_reviews.sql      # Additional review seed data
|   |-- 006_car_multiple_photos.sql # Multi-photo support for cars
|   |-- 007_massive_mock_data.sql # Large set of mock cars
|   |-- 008_partnership.sql       # Partner applications table
|   |-- 009_model_specific_photos.sql # Better car photos by model
|   |-- 010_1000_mock_cars.sql    # Even more mock data
|   |-- 011_chat.sql              # Conversations and messages tables
|   |-- 012_host_dashboard.sql    # Views tracking, dashboard indexes
|   |-- 013_stories.sql           # Instagram-style stories table
|   |-- 014_password_reset_and_damage_reports.sql # Reset tokens, damage reports
|   |-- 015_compliance.sql        # NDPA compliance, audit log, tax records
|   |-- 016_waitlist.sql          # Pre-launch waitlist table
|
|-- src/
|   |-- main.rs                   # Entry point: server setup, routing, middleware
|   |
|   |-- handlers/                 # Request handlers (like controllers)
|   |   |-- mod.rs                # Declares all handler modules
|   |   |-- auth.rs               # Signup, signin, JWT, profile, password reset
|   |   |-- cars.rs               # Car CRUD, search, homepage feed
|   |   |-- bookings.rs           # Booking creation, status transitions
|   |   |-- payments.rs           # Paystack, wallet, withdrawals, banks
|   |   |-- chat.rs               # REST chat endpoints
|   |   |-- ws.rs                 # WebSocket real-time messaging
|   |   |-- reviews.rs            # Review creation, ratings
|   |   |-- dashboard.rs          # Host dashboard statistics
|   |   |-- upload.rs             # File upload (multipart)
|   |   |-- admin.rs              # Admin panel endpoints
|   |   |-- favorites.rs          # Favorite cars toggle
|   |   |-- notifications.rs      # Notification listing, read marking
|   |   |-- cards.rs              # Saved payment cards
|   |   |-- verification.rs       # Email verification codes
|   |   |-- partner.rs            # Partnership applications
|   |   |-- stories.rs            # Instagram-style host stories
|   |   |-- damage_reports.rs     # Vehicle condition reports
|   |   |-- compliance.rs         # NDPA compliance, data export
|   |   |-- waitlist.rs           # Pre-launch waitlist
|   |   |-- health.rs             # Health check endpoint
|   |   |-- protection_plans.rs   # Insurance plan listing
|   |
|   |-- middleware/
|   |   |-- mod.rs                # Declares middleware modules
|   |   |-- auth.rs               # JWT extraction and validation
|   |
|   |-- models/                   # Data structures (like DTOs/entities)
|   |   |-- mod.rs                # Declares all model modules + re-exports
|   |   |-- user.rs               # User, UserPublic, Claims, auth request types
|   |   |-- car.rs                # Car, CreateCarRequest, search queries
|   |   |-- booking.rs            # Booking, BookingWithCar, actions
|   |   |-- payment.rs            # Payment, Paystack types, wallet types
|   |   |-- review.rs             # Review, rating summary
|   |   |-- protection_plan.rs    # ProtectionPlan
|   |   |-- notification.rs       # Notification
|   |   |-- favorite.rs           # Favorite
|   |   |-- card.rs               # SavedCard, SavedCardPublic
|   |   |-- partner.rs            # PartnerApplication
|   |
|   |-- services/
|       |-- mod.rs                # AppConfig (environment variables)
|       |-- email.rs              # Email service (Resend API)
|
|-- uploads/                      # User-uploaded files (photos, voice notes)
|
|-- webapp/                       # React frontend (separate)
|-- mobile/                       # Flutter mobile app (separate)
```

## 2.2 How Cargo.toml Works

`Cargo.toml` is Rust's equivalent of `package.json`. Let me explain every dependency:

```toml
[package]
name = "qent"           # Project name
version = "0.1.0"       # Semantic version
edition = "2021"        # Rust edition (like ECMAScript version)

[dependencies]
# --- Web Framework ---
actix-web = "4"         # The web framework (like Express.js)
actix-rt = "2"          # Actix async runtime
actix-cors = "0.7"      # CORS middleware

# --- Serialization ---
serde = { version = "1", features = ["derive"] }  # Serialize/Deserialize traits
serde_json = "1"        # JSON parsing and generation

# --- Database ---
sqlx = { version = "0.8", features = [
    "runtime-tokio",    # Use tokio as async runtime
    "postgres",         # PostgreSQL driver
    "chrono",           # Date/time type support
    "uuid",             # UUID type support
    "migrate",          # Run migrations from code
    "tls-rustls"        # TLS support for secure DB connections
]}

# --- Async Runtime ---
tokio = { version = "1", features = ["full"] }  # Async runtime (like Node's event loop)

# --- Date/Time ---
chrono = { version = "0.4", features = ["serde"] }  # Date/time with JSON support

# --- UUIDs ---
uuid = { version = "1", features = ["v4", "serde"] }  # UUID generation + serialization

# --- Authentication ---
jsonwebtoken = "9"      # JWT encoding/decoding
bcrypt = "0.16"         # Password hashing

# --- Configuration ---
dotenv = "0.15"         # Load .env files
env_logger = "0.11"     # Logging with RUST_LOG env var
log = "0.4"             # Logging macros (log::info!, log::error!)

# --- Validation ---
validator = { version = "0.19", features = ["derive"] }  # Request validation

# --- HTTP Client ---
reqwest = { version = "0.12", features = ["json"] }  # Make HTTP requests (to Paystack, Resend)

# --- Cryptography ---
hmac = "0.12"           # HMAC for webhook signature verification
sha2 = "0.10"           # SHA-512 for Paystack webhook
hex = "0.4"             # Hex encoding for signature comparison

# --- Utilities ---
rand = "0.8"            # Random number generation (tokens)

# --- Rate Limiting ---
actix-governor = "0.6"  # Rate limiting middleware
governor = "0.7"        # Rate limiting algorithm

# --- WebSocket ---
actix-web-actors = "4"  # WebSocket actor support
actix = "0.13"          # Actor framework

# --- File Upload ---
actix-multipart = "0.7" # Multipart form data parsing
actix-files = "0.6"     # Serve static files (uploaded images)
futures-util = "0.3"    # Stream utilities for multipart processing
```

### What are "features"?

Rust crates (packages) can have optional features. For example, `sqlx` supports multiple databases, but you only enable `postgres` for this project. This keeps your binary small -- unused code is not compiled.

```toml
sqlx = { version = "0.8", features = ["postgres", "uuid"] }
# Only compiles PostgreSQL driver and UUID support
# MySQL, SQLite, etc. are NOT compiled
```

## 2.3 The Module System

Rust's module system determines what code can see what. It is quite different from JavaScript's import/export.

### How `mod` works

In `src/main.rs`:
```rust
mod handlers;     // "Load the handlers module"
mod middleware;    // "Load the middleware module"
mod models;       // "Load the models module"
mod services;     // "Load the services module"
```

When Rust sees `mod handlers;`, it looks for either:
- `src/handlers.rs` (single file module), OR
- `src/handlers/mod.rs` (directory module)

Since `src/handlers/mod.rs` exists, Rust loads it. Inside `mod.rs`:

```rust
pub mod auth;        // Makes src/handlers/auth.rs available
pub mod cars;        // Makes src/handlers/cars.rs available
pub mod bookings;    // Makes src/handlers/bookings.rs available
// ... etc
```

The `pub` keyword makes these modules public -- without it, they would only be visible within the `handlers` module itself.

### How `use` works

`use` brings items into scope, like JavaScript's `import`:

```rust
// JavaScript:
import { PgPool } from 'sqlx';

// Rust:
use sqlx::PgPool;
```

In `src/handlers/auth.rs`:
```rust
use crate::models::{
    AuthResponse, AuthResponseWithRefresh, Claims, ForgotPasswordRequest,
    RefreshTokenRequest, ResetPasswordRequest, SignInRequest, SignUpRequest,
    UpdateProfileRequest, User, UserPublic, VerificationStatus, VerifyIdentityRequest,
};
```

- `crate::` means "start from the root of this project" (like `@/` in some JS setups)
- `models::` is the models module
- The `{...}` imports multiple items at once

### The re-export pattern in models/mod.rs

```rust
pub mod user;
pub mod car;
// ...

pub use user::*;   // Re-export everything from user.rs at the models level
pub use car::*;    // Re-export everything from car.rs at the models level
```

This is why other files can write `use crate::models::User` instead of `use crate::models::user::User`. The `pub use *` re-exports all public items.

---

# Part 3: Actix-Web Framework Deep Dive

## 3.1 What is Actix-Web?

Actix-Web is one of the fastest web frameworks in any language. Some comparisons:

| Feature | Express.js (Node) | Flask (Python) | Actix-Web (Rust) |
|---------|-------------------|----------------|-------------------|
| Speed | ~15,000 req/s | ~5,000 req/s | ~500,000+ req/s |
| Type Safety | None (runtime) | None (runtime) | Full (compile-time) |
| Memory | GC-managed | GC-managed | Zero-cost, no GC |
| Async | Single thread event loop | WSGI (sync) | Multi-thread async |

## 3.2 App Builder Pattern

The entire application is configured in `src/main.rs` using a builder pattern. Let me walk through every line:

```rust
#[actix_web::main]                    // 1. Set up async runtime
async fn main() -> std::io::Result<()> {
    dotenv::dotenv().ok();            // 2. Load .env file
    env_logger::init();               // 3. Initialize logging

    let config = AppConfig::from_env(); // 4. Read all config
    let bind_addr = format!("{}:{}", config.host, config.port);  // 5. Build address string

    // 6. Create database connection pool
    let pool = PgPoolOptions::new()
        .max_connections(10)           // Max 10 simultaneous DB connections
        .connect(&config.database_url)
        .await
        .expect("Failed to create database pool");  // Crash if DB unreachable

    // 7. Run pending migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    // 8. Spawn background task
    let bg_pool = pool.clone();
    tokio::spawn(auto_complete_bookings(bg_pool));

    // 9. Configure rate limiters
    let auth_rate_limit = GovernorConfigBuilder::default()
        .seconds_per_request(6)        // 1 request per 6 seconds baseline
        .burst_size(10)                // But allow bursts of 10
        .finish()
        .unwrap();

    // 10. Start WebSocket manager actor
    let ws_manager = handlers::ws::WsManager::new().start();

    // 11. Build and start HTTP server
    HttpServer::new(move || {          // Called for EACH worker thread
        // 12. Configure CORS
        let cors = Cors::default()
            .allowed_origin("https://qent.online")
            // ... more origins ...
            .allow_any_method()
            .allow_any_header()
            .max_age(3600);

        // 13. Build the application
        App::new()
            .wrap(cors)                           // Apply CORS middleware
            .wrap(Logger::default())              // Log every request
            .app_data(web::Data::new(pool.clone()))  // Share DB pool
            .app_data(web::Data::new(config.clone())) // Share config
            .app_data(web::Data::new(ws_manager.clone())) // Share WS manager
            .route("/health", web::get().to(handlers::health::health_check))
            .route("/ws", web::get().to(handlers::ws::ws_connect))
            .service(actix_files::Files::new("/uploads", "uploads"))
            .service(web::scope("/api")
                // ... all routes ...
            )
    })
    .bind(&bind_addr)?                // 14. Bind to address
    .run()                            // 15. Start serving
    .await
}
```

### The request lifecycle:

```
Client Request
     |
     v
+--------------------+
|  HttpServer        |  (accepts TCP connections)
+--------------------+
     |
     v
+--------------------+
|  CORS Middleware    |  (checks Origin header)
+--------------------+
     |
     v
+--------------------+
|  Logger Middleware  |  (logs method, path, status, time)
+--------------------+
     |
     v
+--------------------+
|  Route Matching    |  (finds the right handler)
+--------------------+
     |
     +---> Public route? ----> Handler directly
     |
     +---> Authenticated route?
              |
              v
          +--------------------+
          |  auth_mw           |  (validates JWT token)
          +--------------------+
              |
              v
          Handler function
              |
              v
          HttpResponse
```

## 3.3 Routing

Actix-Web routing uses a combination of `web::scope` (for path prefixes) and `web::get/post/put/delete` (for HTTP methods).

### Route hierarchy in Qent:

```
/health                              -> health_check (public)
/ws                                  -> ws_connect (WebSocket, token in query)
/uploads/{file}                      -> static file serving
/api/
  /auth/                             -> RATE LIMITED scope
    POST /signup                     -> sign_up
    POST /signin                     -> sign_in
    POST /refresh                    -> refresh_token
    POST /forgot-password            -> forgot_password
    POST /reset-password             -> reset_password
    POST /send-code                  -> send_code
    POST /verify-code                -> verify_code
  GET  /cars/search                  -> search_cars (public)
  GET  /cars/homepage                -> get_homepage (public)
  POST /cars/{id}/view               -> increment_view (public)
  GET  /cars/{id}                    -> get_car (public)
  GET  /protection-plans             -> list_plans (public)
  GET  /users/{id}                   -> get_user_public (public)
  GET  /users/{id}/reviews           -> get_user_reviews (public)
  GET  /users/{id}/rating            -> get_user_rating (public)
  GET  /payments/banks               -> list_banks (public)
  POST /payments/verify-account      -> verify_bank_account (public)
  POST /payments/webhook             -> paystack_webhook (public, verified by HMAC)
  POST /waitlist                     -> join_waitlist (public)
  GET  /waitlist/count               -> waitlist_count (public)
  /                                  -> AUTHENTICATED scope (all routes below need JWT)
    GET  /profile                    -> get_profile
    PUT  /profile                    -> update_profile
    POST /profile/verify-identity    -> verify_identity
    POST /cars                       -> create_car
    GET  /cars/my-listings           -> get_host_cars
    PUT  /cars/{id}                  -> update_car
    POST /cars/{id}/deactivate       -> deactivate_car
    GET  /cars/{id}/booked-dates     -> get_booked_dates
    GET  /dashboard/stats            -> get_host_stats
    GET  /dashboard/listings         -> get_host_listings
    POST /bookings                   -> create_booking
    GET  /bookings/mine              -> get_my_bookings
    GET  /bookings/{id}              -> get_booking
    POST /bookings/{id}/action       -> update_booking_status
    GET  /bookings/host/pending      -> get_host_pending_bookings
    /payments/
      GET  /wallet                   -> get_wallet_balance
      GET  /wallet/transactions      -> get_wallet_transactions
      GET  /earnings                 -> get_earnings
      POST /initiate                 -> initiate_payment
      POST /withdraw                 -> withdraw
      POST /refund/{id}              -> request_refund
    GET  /cards                      -> list_cards
    POST /cards/{id}/default         -> set_default_card
    DELETE /cards/{id}               -> delete_card
    POST /cards/charge               -> charge_saved_card
    POST /reviews                    -> create_review
    GET  /favorites                  -> get_favorites
    POST /favorites/{id}             -> toggle_favorite
    GET  /favorites/{id}/check       -> check_favorite
    GET  /notifications              -> get_notifications
    POST /notifications/{id}/read    -> mark_read
    POST /notifications/read-all     -> mark_all_read
    POST /partner/apply              -> apply
    GET  /partner/application        -> get_application
    GET  /partner/dashboard          -> dashboard
    POST /partner/activate-car       -> activate_car
    GET  /stories                    -> get_stories
    POST /stories                    -> create_story
    DELETE /stories/{id}             -> delete_story
    POST /chat/conversations         -> get_or_create_conversation
    GET  /chat/conversations         -> get_conversations
    GET  /chat/conversations/{id}/messages  -> get_messages
    POST /chat/conversations/{id}/messages  -> send_message
    POST /chat/conversations/{id}/read      -> mark_read
    DELETE /chat/conversations/{id}         -> delete_conversation
    POST /upload                     -> upload_file
    POST /auth/accept-terms          -> accept_terms
    GET  /auth/terms-status          -> terms_status
    POST /account/request-deletion   -> request_deletion
    POST /account/cancel-deletion    -> cancel_deletion
    GET  /account/export             -> export_data
    POST /damage-reports             -> create_report
    GET  /damage-reports/{id}        -> get_reports
    /admin/ (all require admin role check in handler)
      GET  /users                    -> list_users
      POST /users/{id}/verify        -> verify_user
      ... (many more admin routes)
```

### How scopes and middleware interact:

```rust
web::scope("/api")
    // These routes are PUBLIC (no auth middleware):
    .service(
        web::scope("/auth")
            .wrap(Governor::new(&auth_rate_limit))  // But they ARE rate-limited
            .route("/signup", web::post().to(handlers::auth::sign_up))
    )
    .route("/cars/search", web::get().to(handlers::cars::search_cars))

    // These routes are AUTHENTICATED (auth middleware applied):
    .service(
        web::scope("")
            .wrap(actix_web::middleware::from_fn(auth_mw))  // JWT required
            .route("/profile", web::get().to(handlers::auth::get_profile))
    )
```

**Key insight:** Middleware wraps a *scope*. Any route inside that scope has the middleware applied. Routes outside the scope do not.

## 3.4 Extractors: How Handlers Get Data

Extractors are Actix-Web's way of providing data to handler functions. Each parameter type in a handler function is an extractor:

```rust
pub async fn create_booking(
    req: HttpRequest,              // Raw request (for reading extensions)
    pool: web::Data<PgPool>,       // Database connection pool (app state)
    body: web::Json<CreateBookingRequest>,  // Parsed JSON body
) -> HttpResponse {
```

Here is every extractor type used in Qent:

### web::Json<T> -- Parse JSON Request Body

```rust
// The request body is automatically parsed into CreateBookingRequest
pub async fn create_booking(body: web::Json<CreateBookingRequest>) -> HttpResponse {
    // body.car_id, body.start_date, etc. are available
    // If JSON is malformed or missing required fields, Actix returns 400 automatically
}
```

Behind the scenes, Actix:
1. Reads the request body bytes
2. Deserializes them using `serde_json::from_slice::<CreateBookingRequest>()`
3. If deserialization fails, returns 400 Bad Request automatically
4. If it succeeds, wraps the result in `web::Json` and passes it to your handler

### web::Path<T> -- Extract from URL Path

```rust
.route("/cars/{id}", web::get().to(handlers::cars::get_car))

pub async fn get_car(path: web::Path<Uuid>) -> HttpResponse {
    let car_id = path.into_inner();  // Extract the Uuid from the path
}
```

For multiple path parameters:
```rust
.route("/users/{user_id}/reviews/{review_id}", ...)

pub async fn get_review(path: web::Path<(Uuid, Uuid)>) -> HttpResponse {
    let (user_id, review_id) = path.into_inner();
}
```

### web::Query<T> -- Extract from Query String

```rust
.route("/cars/search", web::get().to(handlers::cars::search_cars))

// URL: /api/cars/search?location=Lagos&min_price=10000&sort_by=price_asc

pub async fn search_cars(query: web::Query<CarSearchQuery>) -> HttpResponse {
    // query.location = Some("Lagos")
    // query.min_price = Some(10000.0)
    // query.sort_by = Some("price_asc")
    // All other fields are None
}
```

The `CarSearchQuery` struct uses `Option<T>` for every field because query parameters are optional:
```rust
pub struct CarSearchQuery {
    pub location: Option<String>,
    pub min_price: Option<f64>,
    pub max_price: Option<f64>,
    pub sort_by: Option<String>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
    // ...
}
```

### web::Data<T> -- Shared Application State

```rust
pub async fn sign_up(
    pool: web::Data<PgPool>,        // Database pool
    config: web::Data<AppConfig>,    // App configuration
) -> HttpResponse {
    // pool.get_ref() returns &PgPool
}
```

`web::Data` is how shared state (database pool, config, etc.) is injected into handlers. It is set up in `main.rs`:
```rust
App::new()
    .app_data(web::Data::new(pool.clone()))    // Available as web::Data<PgPool>
    .app_data(web::Data::new(config.clone()))   // Available as web::Data<AppConfig>
```

### HttpRequest -- The Raw Request

```rust
pub async fn get_profile(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    // Used to access extensions (where auth middleware stores Claims)
    let claims = req.extensions().get::<Claims>().cloned();
}
```

### web::Bytes -- Raw Request Body

```rust
pub async fn paystack_webhook(
    req: HttpRequest,
    body_bytes: web::Bytes,        // Raw bytes (needed for HMAC verification)
) -> HttpResponse {
    // body_bytes is the raw POST body
    // We need raw bytes to verify the Paystack signature
}
```

### Multipart -- File Uploads

```rust
pub async fn upload_file(
    req: HttpRequest,
    mut payload: Multipart,        // Multipart form data stream
) -> HttpResponse {
    while let Some(Ok(mut field)) = payload.next().await {
        // Process each field/file in the multipart upload
    }
}
```

### web::Payload -- WebSocket Upgrade

```rust
pub async fn ws_connect(
    req: HttpRequest,
    stream: web::Payload,          // Used for WebSocket upgrade
) -> Result<HttpResponse, Error> {
    ws::start(session, &req, stream)
}
```

## 3.5 Middleware: How auth_mw Works

Middleware intercepts requests before they reach handlers. Qent uses Actix-Web's `from_fn` middleware, which is the simplest way to write middleware:

```
                     +-----------+
  Incoming Request   |           |   Next middleware
  =================> | auth_mw   | ==================>  Handler
                     |           |
                     +-----------+
                          |
                     Token invalid?
                          |
                          v
                     Return 401
                     (short-circuit)
```

Here is the full middleware from `src/main.rs`, line 89-99:

```rust
async fn auth_mw(
    req: ServiceRequest,                      // The incoming request
    next: Next<impl MessageBody>,             // The "rest of the pipeline"
) -> Result<ServiceResponse<impl MessageBody>, Error> {
    // Step 1: Get JWT secret from app config
    let jwt_secret = req
        .app_data::<web::Data<AppConfig>>()   // Access shared state
        .map(|c| c.jwt_secret.clone())        // Get the secret
        .unwrap_or_default();                 // Default to "" if not found

    // Step 2: Extract and validate JWT token
    validate_token(&req, &jwt_secret)?;       // ? = return 401 if invalid

    // Step 3: Continue to the actual handler
    next.call(req).await
}
```

And `validate_token` in `src/middleware/auth.rs`:

```rust
pub fn validate_token(req: &ServiceRequest, jwt_secret: &str) -> Result<(), Error> {
    let claims = extract_claims(req, jwt_secret)?;  // Parse JWT
    req.extensions_mut().insert(claims);              // Store in request extensions
    Ok(())
}

pub fn extract_claims(req: &ServiceRequest, jwt_secret: &str) -> Result<Claims, Error> {
    // Step 1: Get "Authorization" header
    let auth_header = req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ErrorUnauthorized("Missing authorization header"))?;

    // Step 2: Strip "Bearer " prefix
    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ErrorUnauthorized("Invalid authorization format"))?;

    // Step 3: Decode and validate JWT
    let token_data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(jwt_secret.as_bytes()),
        &Validation::default(),    // Checks expiration, algorithm, etc.
    )
    .map_err(|_| ErrorUnauthorized("Invalid token"))?;

    Ok(token_data.claims)
}
```

**How handlers access the Claims:**

After the middleware stores claims in extensions, handlers retrieve them:
```rust
pub async fn get_profile(req: HttpRequest, pool: web::Data<PgPool>) -> HttpResponse {
    let claims = req.extensions().get::<Claims>().cloned();
    let claims = match claims {
        Some(c) => c,
        None => return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "Unauthorized"})),
    };

    // Now claims.sub is the user's UUID
    // And claims.role is their role (Renter/Host/Admin)
}
```

## 3.6 Response Types

Actix-Web provides a builder pattern for responses:

```rust
// 200 OK with JSON body
HttpResponse::Ok().json(user)

// 201 Created with JSON body
HttpResponse::Created().json(booking)

// 400 Bad Request with error message
HttpResponse::BadRequest().json(serde_json::json!({"error": "Invalid input"}))

// 401 Unauthorized
HttpResponse::Unauthorized().json(serde_json::json!({"error": "Unauthorized"}))

// 403 Forbidden
HttpResponse::Forbidden().json(serde_json::json!({"error": "Only hosts can do this"}))

// 404 Not Found
HttpResponse::NotFound().json(serde_json::json!({"error": "Car not found"}))

// 409 Conflict
HttpResponse::Conflict().json(serde_json::json!({"error": "Email already registered"}))

// 500 Internal Server Error
HttpResponse::InternalServerError().json(serde_json::json!({"error": e.to_string()}))
```

The `serde_json::json!` macro creates inline JSON:
```rust
serde_json::json!({"error": "Not found", "details": {"id": car_id}})
// Becomes: {"error":"Not found","details":{"id":"uuid-here"}}
```

## 3.7 CORS Configuration

CORS (Cross-Origin Resource Sharing) controls which domains can make requests to your API.

```rust
let cors = Cors::default()
    .allowed_origin("http://localhost:3000")     // React dev server
    .allowed_origin("http://localhost:5173")     // Vite dev server
    .allowed_origin("http://10.0.2.2:8080")     // Android emulator
    .allowed_origin("https://qent.online")       // Production domain
    .allowed_origin("https://www.qent.online")
    .allowed_origin("https://qent.netlify.app")  // Netlify deployment
    .allow_any_method()                           // GET, POST, PUT, DELETE, etc.
    .allow_any_header()                           // Any request header
    .max_age(3600);                              // Cache preflight for 1 hour
```

**Why so many origins?** Different development environments and deployment targets:
- `localhost:3000/5173` -- local web development
- `10.0.2.2:8080` -- Android emulator accessing host machine
- `qent.online` -- production domain
- `qent.netlify.app` -- Netlify's default subdomain

## 3.8 Rate Limiting with actix-governor

Rate limiting prevents abuse (brute-force login attempts, payment flooding):

```rust
// Auth: 10 requests per minute per IP
let auth_rate_limit = GovernorConfigBuilder::default()
    .seconds_per_request(6)    // Refill 1 token every 6 seconds
    .burst_size(10)            // Bucket holds 10 tokens max
    .finish()
    .unwrap();
```

This uses a "token bucket" algorithm:
- Each IP address has a bucket that holds up to 10 tokens
- One token is added every 6 seconds
- Each request costs 1 token
- If the bucket is empty, the request is rejected with 429 Too Many Requests
- Result: ~10 requests per minute sustained, with bursts up to 10

Applied to a scope:
```rust
web::scope("/auth")
    .wrap(Governor::new(&auth_rate_limit))
    .route("/signup", web::post().to(handlers::auth::sign_up))
    .route("/signin", web::post().to(handlers::auth::sign_in))
```

## 3.9 Static File Serving

```rust
.service(actix_files::Files::new("/uploads", "uploads").show_files_listing())
```

This serves the `uploads/` directory at the `/uploads` URL path. When a user uploads a profile photo, the returned URL might be `/uploads/user-id_uuid.jpg`, and this line makes that file accessible via HTTP.

---

# Part 4: Database with SQLx

## 4.1 What is SQLx?

SQLx is NOT an ORM. It is a compile-time checked SQL toolkit. Unlike ORMs (like Sequelize or TypeORM), you write raw SQL. The difference from other raw SQL libraries:

1. **Compile-time query checking** (optional) -- SQLx can verify your SQL queries against the actual database at compile time
2. **Type-safe results** -- query results are mapped to Rust structs via `FromRow`
3. **Async by default** -- every query is `async`
4. **Zero overhead** -- no query builder abstraction layer

## 4.2 PgPool: Connection Pooling

```rust
let pool = PgPoolOptions::new()
    .max_connections(10)
    .connect(&config.database_url)
    .await
    .expect("Failed to create database pool");
```

A connection pool maintains a set of open database connections. Instead of opening a new TCP connection for every query (slow), the pool reuses connections.

```
                    +--------+
  Handler 1 ------> |        |
  Handler 2 ------> |  Pool  | ----> [Conn 1] ----> PostgreSQL
  Handler 3 ------> | (max   | ----> [Conn 2] ----> PostgreSQL
  Handler 4 ------> |  10)   | ----> [Conn 3] ----> PostgreSQL
  ...                |        |       ...
                    +--------+
```

With `max_connections(10)`, at most 10 queries run simultaneously. If all connections are busy, new queries wait in a queue.

The pool is shared across all handlers via `web::Data`:
```rust
.app_data(web::Data::new(pool.clone()))

// In a handler:
pub async fn get_car(pool: web::Data<PgPool>) -> HttpResponse {
    sqlx::query_as::<_, Car>("SELECT * FROM cars WHERE id = $1")
        .bind(car_id)
        .fetch_optional(pool.get_ref())  // .get_ref() extracts &PgPool from web::Data
        .await
```

## 4.3 Query Methods: query vs query_as vs query_scalar

### `sqlx::query` -- Execute without mapping to a struct

Used for INSERT, UPDATE, DELETE, or when you don't need results:

```rust
// From src/handlers/auth.rs:
let result = sqlx::query(
    r#"INSERT INTO users (id, email, phone, password_hash, full_name, role, ...)
    VALUES ($1, $2, $3, $4, $5, $6, ...)"#,
)
.bind(id)                    // $1 = id
.bind(&body.email)           // $2 = email
.bind(&body.phone)           // $3 = phone
.bind(&password_hash)        // $4 = password_hash
.bind(&body.full_name)       // $5 = full_name
.bind(crate::models::UserRole::Renter)  // $6 = role
.execute(pool.get_ref())     // Run the query, return affected rows
.await;
```

### `sqlx::query_as::<_, T>` -- Map rows to a struct

Used for SELECT when you want typed results:

```rust
// From src/handlers/auth.rs:
let user = sqlx::query_as::<_, User>(
    "SELECT * FROM users WHERE LOWER(email) = LOWER($1) AND is_active = true",
)
.bind(&body.email)
.fetch_optional(pool.get_ref())   // Returns Result<Option<User>, sqlx::Error>
.await;
```

The `<_, User>` means: "The database type is inferred (_), and map results to User struct."

For this to work, `User` must derive `FromRow`:
```rust
#[derive(FromRow)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    // ... every column must have a matching field
}
```

### `sqlx::query_scalar::<_, T>` -- Get a single value

Used when your query returns one column and one row:

```rust
// Check if email exists (returns bool):
let existing = sqlx::query_scalar::<_, bool>(
    "SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)"
)
.bind(&body.email)
.fetch_one(pool.get_ref())
.await;

// Get wallet balance (returns f64):
let balance = sqlx::query_scalar::<_, f64>(
    "SELECT wallet_balance FROM users WHERE id = $1"
)
.bind(claims.sub)
.fetch_one(pool.get_ref())
.await;
```

## 4.4 Fetch Methods

| Method | Returns | Use When |
|--------|---------|----------|
| `.fetch_one()` | `Result<T, Error>` | You expect exactly one row (error if 0 or 2+) |
| `.fetch_optional()` | `Result<Option<T>, Error>` | Zero or one row |
| `.fetch_all()` | `Result<Vec<T>, Error>` | Zero or more rows |
| `.execute()` | `Result<PgQueryResult, Error>` | INSERT/UPDATE/DELETE (no row data needed) |

**Common pattern in Qent** -- fetch_optional with match:

```rust
let user = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
    .bind(user_id)
    .fetch_optional(pool.get_ref())
    .await;

match user {
    Ok(Some(u)) => HttpResponse::Ok().json(u),      // Found
    Ok(None) => HttpResponse::NotFound().json(...),   // Not found
    Err(e) => HttpResponse::InternalServerError().json(...),  // DB error
}
```

## 4.5 Parameter Binding ($1, $2...) -- SQL Injection Prevention

Never concatenate user input into SQL. Always use parameter binding:

```rust
// DANGEROUS -- never do this:
let sql = format!("SELECT * FROM users WHERE email = '{}'", user_input);

// SAFE -- always do this:
sqlx::query("SELECT * FROM users WHERE email = $1")
    .bind(&user_input)   // $1 is replaced safely
```

Parameters are numbered: `$1`, `$2`, `$3`, etc. Each `.bind()` call fills the next parameter in order.

```rust
sqlx::query(
    "UPDATE users SET full_name = $1, phone = $2 WHERE id = $3"
)
.bind(&body.full_name)     // $1
.bind(&body.phone)         // $2
.bind(claims.sub)          // $3
.execute(pool.get_ref())
.await;
```

## 4.6 Migrations: Schema Evolution

Migrations are SQL files that create and modify your database schema. They run in alphabetical/numerical order.

### Migration files in Qent:

**001_initial.sql** -- The foundation:
```sql
-- Create custom enum types
CREATE TYPE user_role AS ENUM ('renter', 'host', 'admin');
CREATE TYPE verification_status AS ENUM ('pending', 'verified', 'rejected');
CREATE TYPE car_status AS ENUM ('active', 'inactive', 'pendingapproval', 'rejected');
CREATE TYPE booking_status AS ENUM ('pending', 'approved', 'rejected', 'confirmed',
                                     'active', 'completed', 'cancelled');
CREATE TYPE payment_status AS ENUM ('pending', 'success', 'failed', 'refunded');
CREATE TYPE transaction_type AS ENUM ('payment', 'payout', 'refund');
CREATE TYPE plan_tier AS ENUM ('basic', 'standard', 'premium');

-- Tables: users, cars, protection_plans, bookings, payments,
--         wallet_transactions, reviews
-- Plus indexes for performance
```

**002_additions_and_seed.sql** -- Additional tables and mock data:
```sql
ALTER TABLE users ADD COLUMN IF NOT EXISTS country VARCHAR(100) DEFAULT 'Nigeria';
ALTER TABLE cars ADD COLUMN IF NOT EXISTS seats INTEGER DEFAULT 5;

-- New tables: favorites, notifications, verification_codes
-- Mock data: 6 users, 8 cars, 3 bookings, reviews, notifications
```

**003_saved_cards.sql** -- Paystack card tokenization:
```sql
CREATE TABLE saved_cards (
    id UUID PRIMARY KEY,
    user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    authorization_code VARCHAR(255) NOT NULL,  -- Paystack reuse token
    card_type VARCHAR(50) NOT NULL,
    last4 VARCHAR(4) NOT NULL,
    -- ...
);
-- Unique constraint: only one default card per user
CREATE UNIQUE INDEX idx_saved_cards_default
    ON saved_cards(user_id) WHERE is_default = true;
```

**011_chat.sql** -- Real-time messaging:
```sql
CREATE TABLE conversations (
    id UUID PRIMARY KEY,
    car_id UUID REFERENCES cars(id),
    renter_id UUID REFERENCES users(id),
    host_id UUID REFERENCES users(id),
    last_message_text TEXT DEFAULT '',
    last_message_at TIMESTAMP DEFAULT NOW(),
    renter_unread_count INTEGER DEFAULT 0,
    host_unread_count INTEGER DEFAULT 0,
    -- Unique constraint: one conversation per car-renter pair
    UNIQUE(car_id, renter_id)
);

CREATE TABLE messages (
    id UUID PRIMARY KEY,
    conversation_id UUID REFERENCES conversations(id) ON DELETE CASCADE,
    sender_id UUID REFERENCES users(id),
    content TEXT NOT NULL,
    message_type VARCHAR(20) DEFAULT 'text',  -- text, image, voice, location
    reply_to_id UUID REFERENCES messages(id), -- Thread replies
    is_read BOOLEAN DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);
```

**014_password_reset_and_damage_reports.sql**:
```sql
CREATE TABLE password_reset_tokens (
    id UUID PRIMARY KEY,
    user_id UUID REFERENCES users(id),
    token VARCHAR(64) NOT NULL UNIQUE,
    expires_at TIMESTAMP NOT NULL,
    used BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE TABLE damage_reports (
    id UUID PRIMARY KEY,
    booking_id UUID REFERENCES bookings(id),
    reporter_id UUID REFERENCES users(id),
    reporter_role VARCHAR(10) NOT NULL,     -- 'host' or 'renter'
    photos TEXT[] NOT NULL DEFAULT '{}',
    notes TEXT,
    odometer_reading INTEGER,
    fuel_level VARCHAR(20),
    exterior_condition VARCHAR(20) DEFAULT 'good',
    interior_condition VARCHAR(20) DEFAULT 'good',
    confirmed BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Add fields to existing tables:
ALTER TABLE wallet_transactions ADD COLUMN status VARCHAR(20) DEFAULT 'completed';
ALTER TABLE wallet_transactions ADD COLUMN admin_notes TEXT;
ALTER TABLE users ADD COLUMN refresh_token VARCHAR(128);
```

**015_compliance.sql** -- NDPA (Nigeria Data Protection Act) compliance:
```sql
ALTER TABLE users ADD COLUMN tos_accepted_at TIMESTAMP;
ALTER TABLE users ADD COLUMN privacy_accepted_at TIMESTAMP;
ALTER TABLE users ADD COLUMN tos_version VARCHAR(10);

CREATE TABLE deletion_requests (...);   -- GDPR/NDPA right to delete
CREATE TABLE audit_log (...);           -- Track sensitive actions
CREATE TABLE tax_records (...);         -- Withholding tax tracking
```

### How migrations run:

In `main.rs`:
```rust
sqlx::migrate!("./migrations")
    .run(&pool)
    .await
    .expect("Failed to run migrations");
```

SQLx tracks which migrations have already run in a `_sqlx_migrations` table. On each server start, it checks for new migration files and runs them in order. Already-run migrations are skipped.

**To create a new migration:**
1. Create a file: `migrations/017_new_feature.sql`
2. Write your SQL
3. Restart the server (or run `sqlx migrate run`)

## 4.7 The FromRow Derive Macro

`FromRow` automatically maps database columns to struct fields:

```rust
#[derive(FromRow)]
pub struct Car {
    pub id: Uuid,              // maps to column "id" (type UUID)
    pub host_id: Uuid,         // maps to column "host_id"
    pub make: String,          // maps to column "make" (VARCHAR)
    pub price_per_day: f64,    // maps to column "price_per_day" (DOUBLE PRECISION)
    pub photos: Vec<String>,   // maps to column "photos" (TEXT[])
    pub latitude: Option<f64>, // maps to nullable column "latitude"
}
```

The field name MUST match the column name (or column alias in the query). If your query uses `AS`:
```sql
SELECT c.make || ' ' || c.model AS car_name
```
Then your struct needs:
```rust
pub car_name: String,
```

## 4.8 Database Entity Relationship Diagram

```
+------------------+      +------------------+      +------------------+
|     users        |      |      cars        |      | protection_plans |
+------------------+      +------------------+      +------------------+
| id          UUID |<-+   | id          UUID |   +->| id          UUID |
| email     STRING |  |   | host_id     UUID |---+  | name      STRING |
| phone     STRING |  |   | make      STRING |   |  | tier     ENUM    |
| password  STRING |  |   | model     STRING |   |  | daily_rate FLOAT |
| full_name STRING |  |   | year         INT |   |  | coverage   FLOAT |
| role       ENUM  |  |   | color     STRING |   |  +------------------+
| wallet_bal FLOAT |  |   | plate     STRING |   |
| verification ENUM|  |   | price      FLOAT |   |
| country   STRING |  |   | location  STRING |   |
| refresh_token STR|  |   | lat/lng    FLOAT |   |
+------------------+  |   | photos   TEXT[]  |   |
         |             |   | features TEXT[]  |   |
         |             |   | status     ENUM  |   |
         |             |   | seats        INT |   |
         |             |   | views_count  INT |   |
         |             |   +------------------+   |
         |             |          |                |
         |             |          |                |
         |    +--------+----------+--------+       |
         |    |                            |       |
         |    v                            v       |
+-------------------+           +------------------+
|    bookings       |           |  conversations   |
+-------------------+           +------------------+
| id           UUID |           | id          UUID |
| car_id       UUID |---------->| car_id      UUID |
| renter_id    UUID |--------+  | renter_id   UUID |
| host_id      UUID |------+ |  | host_id     UUID |
| start_date   DATE |      | |  | last_message STR |
| end_date     DATE |      | |  | unread counts INT|
| total_days    INT |      | |  +------------------+
| price_per_day FLT |      | |          |
| subtotal     FLOAT|      | |          v
| protection_id UUID|------+-+  +------------------+
| protection_fee FLT|      | |  |    messages      |
| service_fee  FLOAT|      | |  +------------------+
| total_amount FLOAT|      | |  | id          UUID |
| status       ENUM |      | |  | conversation UUID|
| cancel_reason STR |      | |  | sender_id   UUID |
+-------------------+      | |  | content   STRING |
         |                  | |  | message_type STR |
         |                  | |  | reply_to_id UUID |
    +----+----+             | |  | is_read     BOOL |
    |         |             | |  +------------------+
    v         v             | |
+--------+ +----------+    | |  +------------------+
|payments| | reviews   |    | |  |   favorites      |
+--------+ +----------+    | |  +------------------+
| id UUID| | id    UUID|   | +->| user_id     UUID |
| booking| | booking   |   |    | car_id      UUID |
| payer  | | reviewer  |   |    +------------------+
| amount | | reviewee  |   |
| status | | rating INT|   |    +------------------+
| type   | | comment   |   |    |  notifications   |
+--------+ +----------+    |    +------------------+
                           +--->| user_id     UUID |
+------------------+            | title     STRING |
| wallet_transactions|          | message   STRING |
+------------------+            | type      STRING |
| id          UUID |            | is_read     BOOL |
| user_id     UUID |            | data        JSON |
| amount     FLOAT |            +------------------+
| balance    FLOAT |
| description STR  |       +------------------+
| status      STR  |       |   saved_cards    |
+------------------+       +------------------+
                           | id          UUID |
+------------------+       | user_id     UUID |
| damage_reports   |       | auth_code STRING |
+------------------+       | card_type STRING |
| id          UUID |       | last4     STRING |
| booking_id  UUID |       | bank      STRING |
| reporter_id UUID |       | is_default  BOOL |
| photos    TEXT[] |       +------------------+
| notes     STRING |
| odometer     INT |   +--------------------+
| fuel_level   STR |   | partner_applications|
| ext_condition STR|   +--------------------+
| int_condition STR|   | id            UUID |
| confirmed   BOOL |   | user_id       UUID |
+------------------+   | car details   ...  |
                        | status       STRING|
+------------------+    +--------------------+
|    stories       |
+------------------+    +------------------+
| id          UUID |    |    waitlist      |
| host_id     UUID |    +------------------+
| car_id      UUID |    | email     STRING |
| image_url STRING |    | phone     STRING |
| caption   STRING |    | name      STRING |
| expires_at  TIME |    | role      STRING |
+------------------+    | city      STRING |
                        +------------------+
+------------------+
| password_reset   |    +------------------+
|    _tokens       |    |   audit_log      |
+------------------+    +------------------+
| user_id     UUID |    | user_id     UUID |
| token     STRING |    | action    STRING |
| expires_at  TIME |    | ip_address  STR  |
| used        BOOL |    | details    JSON  |
+------------------+    +------------------+
```

---

# Part 5: Authentication & Security

## 5.1 Password Hashing with bcrypt

When a user signs up, their password is hashed before storage:

```rust
// src/handlers/auth.rs, sign_up function:
let password_hash = match hash(&body.password, DEFAULT_COST) {
    Ok(h) => h,
    Err(_) => return HttpResponse::InternalServerError()
        .json(serde_json::json!({"error": "Failed to hash password"})),
};
```

- `hash(&body.password, DEFAULT_COST)` -- takes the plain-text password and a cost factor
- `DEFAULT_COST` is 12, meaning 2^12 = 4096 iterations
- The result looks like: `$2b$12$LJ3m4ys3Lk0TSwHjlS5P0u8bVGQk7RRj6b6bCwFbNqGqQ3kZ1xFa6`
- This hash includes the salt (random data), so two users with the same password get different hashes

During sign-in, the password is verified:
```rust
if !verify(&body.password, &user.password_hash).unwrap_or(false) {
    return HttpResponse::Unauthorized()
        .json(serde_json::json!({"error": "Invalid credentials"}));
}
```

`verify()` takes the plain-text password and the stored hash, and returns `true` if they match. It extracts the salt from the stored hash and re-hashes the input to compare.

**Security properties:**
- One-way: you cannot reverse a hash back to the password
- Salted: same password produces different hashes
- Slow by design: prevents brute-force attacks (4096 iterations)

## 5.2 JWT Tokens: Claims Structure

JWT (JSON Web Token) is used for stateless authentication. After login, the client receives a token that contains:

```rust
// src/models/user.rs:
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claims {
    pub sub: Uuid,          // "subject" -- the user's ID
    pub role: UserRole,     // renter, host, or admin
    pub exp: usize,         // Expiration timestamp (Unix epoch)
}
```

Token creation:
```rust
let claims = Claims {
    sub: user.id,
    role: user.role.clone(),
    exp: (Utc::now() + chrono::Duration::hours(24)).timestamp() as usize,
};

let token = encode(
    &Header::default(),                              // Algorithm: HS256
    &claims,
    &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
).unwrap();
```

The resulting JWT looks like: `eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIuLi4ifQ.signature`

It has three parts (base64-encoded):
1. **Header**: `{"alg":"HS256"}` -- signing algorithm
2. **Payload**: `{"sub":"user-uuid","role":"host","exp":1711000000}` -- the Claims
3. **Signature**: HMAC-SHA256 of header+payload using JWT_SECRET

Token decoding (in middleware):
```rust
let token_data = decode::<Claims>(
    token,
    &DecodingKey::from_secret(jwt_secret.as_bytes()),
    &Validation::default(),
)
```

`Validation::default()` checks:
- The token's signature is valid (not tampered with)
- The `exp` field is in the future (not expired)
- The algorithm matches (HS256)

## 5.3 Refresh Tokens

JWT access tokens expire after 24 hours. Refresh tokens allow getting a new access token without re-entering credentials:

```
Client                          Server
  |                               |
  |-- POST /auth/signin --------->|
  |<-- {token, refresh_token} ----|
  |                               |
  |   (24 hours later, token expired)
  |                               |
  |-- POST /auth/refresh -------->|
  |   {refresh_token: "abc123"}   |
  |<-- {token, refresh_token} ----|  (new tokens, old refresh token invalidated)
```

The refresh token flow in `src/handlers/auth.rs`:

```rust
pub async fn refresh_token(...) -> HttpResponse {
    // Find user by refresh token
    let user = sqlx::query_as::<_, User>(
        "SELECT * FROM users WHERE refresh_token = $1 AND is_active = true",
    )
    .bind(&body.refresh_token)
    .fetch_optional(pool.get_ref())
    .await;

    // Generate new JWT
    let token = encode(&Header::default(), &claims, &key).unwrap();

    // ROTATE refresh token (old one is invalidated)
    let new_refresh = generate_refresh_token();
    let _ = sqlx::query("UPDATE users SET refresh_token = $1 WHERE id = $2")
        .bind(&new_refresh)
        .bind(user.id)
        .execute(pool.get_ref())
        .await;

    HttpResponse::Ok().json(AuthResponseWithRefresh { token, refresh_token: new_refresh, ... })
}
```

**Token rotation** is a security best practice: each time a refresh token is used, it is replaced with a new one. If an attacker steals a refresh token, the legitimate user's next refresh will fail, alerting them.

## 5.4 The Generate Refresh Token Function

```rust
fn generate_refresh_token() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..64).map(|_| {
        let idx = rng.gen_range(0..36);
        if idx < 10 { (b'0' + idx) as char } else { (b'a' + idx - 10) as char }
    }).collect()
}
```

This generates a 64-character alphanumeric string. Let me break it down:
- `rand::thread_rng()` -- cryptographically secure random number generator
- `(0..64).map(...)` -- generate 64 characters
- `rng.gen_range(0..36)` -- pick a number 0-35
- If 0-9: use digit character ('0'-'9')
- If 10-35: use letter character ('a'-'z')
- `.collect()` -- collect the characters into a String

## 5.5 Route Protection Architecture

The Qent API splits routes into three security levels:

```
Level 1: Public (no auth)
  - GET  /api/cars/search
  - GET  /api/cars/{id}
  - GET  /api/cars/homepage
  - POST /api/auth/signup
  - POST /api/auth/signin
  - POST /api/payments/webhook (verified by HMAC instead)
  - GET  /api/users/{id}
  - GET  /api/users/{id}/reviews
  - GET  /api/payments/banks

Level 2: Rate-limited public (no auth, but rate limited)
  - POST /api/auth/signup      (10 req/min)
  - POST /api/auth/signin      (10 req/min)
  - POST /api/auth/forgot-password

Level 3: Authenticated (JWT required)
  - ALL routes inside the inner web::scope("")
  - These routes check claims.sub for the user ID
  - Some also check claims.role for authorization (admin-only, host-only)
```

**Authorization within handlers:**

Even within authenticated routes, handlers check roles:
```rust
// Only hosts can create cars:
if claims.role != UserRole::Host && claims.role != UserRole::Admin {
    return HttpResponse::Forbidden()
        .json(serde_json::json!({"error": "Only hosts can list cars"}));
}

// Only the host can approve bookings:
if booking.host_id != claims.sub && claims.role != UserRole::Admin {
    return HttpResponse::Forbidden()
        .json(serde_json::json!({"error": "Only the host can approve"}));
}
```

---

# Part 6: Every Handler Explained

## 6.1 Auth Handlers (src/handlers/auth.rs)

### POST /api/auth/signup

**What it does:** Creates a new user account.

**Request:**
```json
{
    "email": "user@example.com",
    "password": "secret123",
    "full_name": "John Doe",
    "phone": "+2348012345678",
    "role": "renter",
    "country": "Nigeria"
}
```

**Flow:**
1. Validate input with the `validator` crate (email format, password length >= 6, name length >= 2)
2. Check if email already exists in database
3. Hash password with bcrypt
4. Generate UUID for new user
5. Insert into `users` table with role=renter, verification_status=pending, wallet_balance=0
6. Create JWT claims (sub=user_id, role=renter, exp=24h from now)
7. Encode JWT token
8. Generate and store refresh token
9. Return 201 Created with `{token, refresh_token, user}`

**Key SQL:**
```sql
INSERT INTO users (id, email, phone, password_hash, full_name, role,
                   verification_status, wallet_balance, is_active, country,
                   created_at, updated_at)
VALUES ($1, $2, $3, $4, $5, $6, $7, 0.0, true, $8, $9, $9)
```

### POST /api/auth/signin

**What it does:** Authenticates a user and returns tokens.

**Flow:**
1. Look up user by email (case-insensitive with `LOWER()`)
2. Only finds active users (`is_active = true`)
3. Verify password against stored bcrypt hash
4. Generate JWT and refresh token
5. Return 200 OK with `{token, refresh_token, user}`

**Security detail:** The `LOWER(email) = LOWER($1)` ensures case-insensitive email matching. A user who registered as "User@Example.com" can sign in with "user@example.com".

### POST /api/auth/refresh

**What it does:** Issues a new JWT using a refresh token.

**Flow:**
1. Find user by refresh_token column
2. Generate new JWT
3. ROTATE refresh token (generate new one, save to DB, return new one)
4. Old refresh token is now invalid

### POST /api/auth/forgot-password

**What it does:** Sends a password reset code via email.

**Security:** Always returns success message regardless of whether the email exists. This prevents email enumeration attacks (attackers cannot determine which emails are registered).

**Flow:**
1. Look up user by email
2. If not found, return success anyway (prevent enumeration)
3. Generate 64-character reset token
4. Invalidate any previous unused tokens for this user
5. Store token with 30-minute expiration
6. Email the first 8 characters as a short code
7. Return generic success message

### POST /api/auth/reset-password

**What it does:** Resets password using the code from the email.

**Flow:**
1. Validate new password length >= 6
2. Find valid token (matches by full token OR first 8 characters)
3. Hash new password
4. Update user's password_hash
5. Mark token as used

### GET /api/profile (authenticated)

**What it does:** Returns the current user's full profile.

**Flow:**
1. Extract Claims from request extensions (set by auth middleware)
2. Query user by `claims.sub` (user ID)
3. Convert User to UserPublic (strips `password_hash`) and return

### PUT /api/profile (authenticated)

**What it does:** Updates profile fields.

Uses `COALESCE($1, full_name)` pattern -- if the new value is NULL, keep the old value. This allows partial updates.

### GET /api/users/{id} (public)

**What it does:** Returns limited public info for any user (name, photo, role). Used in car listings and chat to show user details.

## 6.2 Cars Handlers (src/handlers/cars.rs)

### POST /api/cars (authenticated, host-only)

**What it does:** Creates a new car listing.

**Authorization:** Only users with role Host or Admin.

**Flow:**
1. Check role is Host or Admin
2. Validate input
3. Generate UUID
4. Insert car with status `PendingApproval`
5. Use CTE (WITH clause) to also fetch the host name
6. Return the newly created car

**Key SQL pattern -- CTE (Common Table Expression):**
```sql
WITH inserted AS (
    INSERT INTO cars (...)
    VALUES (...)
    RETURNING *
)
SELECT inserted.*, u.full_name as host_name
FROM inserted
LEFT JOIN users u ON u.id = inserted.host_id
```

This is a powerful PostgreSQL pattern: insert a row and immediately join it with another table in one query.

### GET /api/cars/search (public)

**What it does:** Searches cars with filters, sorting, and pagination.

**Query parameters:**
- `location` -- filter by city (case-insensitive LIKE)
- `min_price`, `max_price` -- price range
- `make`, `model` -- car brand/model
- `start_date`, `end_date` -- availability dates
- `color`, `seats` -- car features
- `sort_by` -- `price_asc`, `price_desc`, `newest`, `rating`, `distance`
- `latitude`, `longitude` -- for distance sorting
- `page`, `per_page` -- pagination

**Dynamic SQL building:**

This handler builds SQL dynamically because the sort order depends on `sort_by`:

```rust
let order_clause = match query.sort_by.as_deref() {
    Some("price_asc") => "c.price_per_day ASC, c.created_at DESC",
    Some("price_desc") => "c.price_per_day DESC, c.created_at DESC",
    Some("newest") => "c.created_at DESC",
    Some("rating") => "COALESCE(rs.avg_rating, 0.0) DESC, c.created_at DESC",
    Some("distance") => {
        if query.latitude.is_some() && query.longitude.is_some() {
            "distance_km ASC NULLS LAST, c.created_at DESC"
        } else {
            "COALESCE(rs.avg_rating, 0.0) DESC, c.created_at DESC"
        }
    }
    _ => "COALESCE(rs.avg_rating, 0.0) DESC, c.created_at DESC",
};
```

**Nullable parameter trick:**

Each filter uses the pattern `AND ($N::type IS NULL OR condition)`:
```sql
AND ($1::text IS NULL OR LOWER(c.location) LIKE LOWER('%' || $1 || '%'))
AND ($2::double precision IS NULL OR c.price_per_day >= $2)
```

If `$1` is NULL (no location filter), the condition is always true (skipped). If `$1` is "Lagos", it filters by location. This avoids building different SQL for different combinations of filters.

**Pagination:**
```sql
LIMIT $10 OFFSET $11
```
- `per_page = query.per_page.unwrap_or(20).min(100)` -- default 20, max 100
- `offset = (page - 1) * per_page`

### GET /api/cars/homepage (public)

**What it does:** Returns categorized car sections for the home feed.

Returns four sections:
1. **Recommended** -- personalized based on booking history (if logged in), or newest cars
2. **Best Cars** -- rating >= 4.0, sorted by rating
3. **Nearby** -- sorted by haversine distance (if coordinates provided)
4. **Popular** -- sorted by trip count

**Haversine formula** (calculates distance between two lat/lng points):
```rust
fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r = 6371.0; // Earth radius in km
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().asin();
    r * c
}
```

**Personalization logic:**

For logged-in users, the recommended section:
1. Queries the user's past bookings to find favorite car makes and average price
2. Filters cars matching those makes or in a price range (50%-150% of average)
3. Sorts by relevance: matching make first, then by rating

### GET /api/cars/{id}/booked-dates (authenticated)

**What it does:** Returns all booked date ranges for a car (used to disable dates in the booking calendar).

```sql
SELECT start_date, end_date FROM bookings
WHERE car_id = $1 AND status IN ('approved', 'confirmed', 'active')
ORDER BY start_date
```

## 6.3 Bookings Handlers (src/handlers/bookings.rs)

### POST /api/bookings (authenticated)

**What it does:** Creates a new booking request.

**Business logic flow:**
1. Fetch the car (must be active)
2. Prevent self-booking (can't book your own car)
3. Check for date overlaps with existing bookings
4. Calculate pricing:
   - `subtotal = price_per_day * total_days`
   - `service_fee = subtotal * 10%`
   - `protection_fee = plan.daily_rate * total_days` (if plan selected)
   - `total_amount = subtotal + service_fee + protection_fee`
5. Insert booking with status `pending`
6. Create notification for the host

**Date overlap check:**
```sql
SELECT EXISTS(
    SELECT 1 FROM bookings
    WHERE car_id = $1
    AND status IN ('pending', 'approved', 'confirmed', 'active')
    AND start_date <= $3 AND end_date >= $2
)
```
This checks if any existing booking's date range overlaps with the requested range. The logic: a booking overlaps if its start is before the requested end AND its end is after the requested start.

### POST /api/bookings/{id}/action (authenticated)

**What it does:** Transitions a booking to a new status.

**State machine:**

```
                    +----------+
                    |  Pending |
                    +----------+
                   /     |      \
                  v      v       v
           +--------+ +--------+ +----------+
           |Approved| |Rejected| |Cancelled |
           +--------+ +--------+ +----------+
                |
                v
           +----------+
           |Confirmed |  (after payment)
           +----------+
                |
                v
           +--------+
           | Active |
           +--------+
                |
                v
           +-----------+
           | Completed |
           +-----------+
```

**Actions and who can perform them:**

| Action | Who | Precondition |
|--------|-----|--------------|
| Approve | Host or Admin | Status = Pending |
| Reject | Host or Admin | Any (but usually Pending) |
| Cancel | Renter, Host, or Admin | Various |
| Activate | Host or Admin | Status = Approved or Confirmed |
| Complete | Host or Admin | Status = Active |

**When a booking is completed:**
```rust
if new_status == BookingStatus::Completed {
    let host_payout = booking.subtotal * 0.85;  // Host gets 85%
    // Credit host wallet
    // Record wallet transaction
}
```

**Notifications:** Every status change triggers a notification to the relevant party, plus an email via the Resend API.

### Helper: create_notification

```rust
async fn create_notification(
    pool: &PgPool,
    user_id: Uuid,
    title: &str,
    message: &str,
    notification_type: &str,
    data: Option<serde_json::Value>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO notifications (id, user_id, title, message,
            notification_type, is_read, data, created_at)
        VALUES ($1, $2, $3, $4, $5, false, $6, NOW())"#,
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(title)
    .bind(message)
    .bind(notification_type)
    .bind(data)
    .execute(pool)
    .await?;
    Ok(())
}
```

## 6.4 Payments Handlers (src/handlers/payments.rs)

(Covered in detail in Part 8)

## 6.5 Chat Handlers (src/handlers/chat.rs)

### POST /api/chat/conversations (authenticated)

**What it does:** Gets or creates a conversation between two users about a car.

**Key logic:** Determines who is the host and who is the renter by checking car ownership:
```rust
let car_host_id = sqlx::query_scalar::<_, Uuid>("SELECT host_id FROM cars WHERE id = $1")
    .bind(car_id)
    .fetch_optional(pool.get_ref())
    .await;

let (renter_id, host_id) = if caller_id == car_host_id {
    (other_user_id, caller_id)    // Caller is the host
} else {
    (caller_id, other_user_id)    // Caller is the renter
};
```

The `UNIQUE(car_id, renter_id)` constraint ensures one conversation per car-renter pair.

### GET /api/chat/conversations (authenticated)

**What it does:** Lists all conversations for the current user with rich data.

The SQL joins conversations with users and cars to return:
- Other user's name and role
- Car name and photo
- Unread counts and last message

### GET /api/chat/conversations/{id}/messages (authenticated)

**What it does:** Gets all messages in a conversation.

**Side effects:**
1. Resets the caller's unread count to 0
2. Marks individual messages as read (where sender != caller)

### POST /api/chat/conversations/{id}/messages (authenticated)

**What it does:** Sends a message in a conversation.

**Side effects:**
1. Inserts the message
2. Updates conversation's `last_message_text` and `last_message_at`
3. Increments the OTHER user's unread count

### DELETE /api/chat/conversations/{id} (authenticated)

**What it does:** Deletes a conversation and all its messages.

Deletes messages first (FK constraint), then the conversation.

## 6.6 Reviews Handler (src/handlers/reviews.rs)

### POST /api/reviews (authenticated)

**What it does:** Creates a review for a completed booking.

**Business rules:**
1. The booking must be completed
2. The reviewer must be a participant (renter or host)
3. No duplicate reviews (UNIQUE constraint on booking_id + reviewer_id)
4. Rating must be 1-5 (validated by `#[validate(range(min = 1, max = 5))]`)

### GET /api/users/{id}/reviews (public)

Returns all reviews where the user is the reviewee, newest first.

### GET /api/users/{id}/rating (public)

Returns aggregate rating: `AVG(rating)` and `COUNT(*)`.

## 6.7 Dashboard Handler (src/handlers/dashboard.rs)

### GET /api/dashboard/stats (authenticated)

**What it does:** Returns comprehensive host statistics in parallel queries.

```rust
// Each of these runs as a separate query:
let total_listings = sqlx::query_scalar("SELECT COUNT(*) FROM cars WHERE host_id = $1")...
let active_listings = sqlx::query_scalar("SELECT COUNT(*) FROM cars WHERE host_id = $1 AND status = 'active'")...
let total_views = sqlx::query_scalar("SELECT COALESCE(SUM(views_count::bigint), 0) FROM cars WHERE host_id = $1")...
let total_bookings = sqlx::query_scalar("SELECT COUNT(*) FROM bookings...")...
let total_earnings = sqlx::query_scalar("SELECT COALESCE(SUM(b.total_price), 0.0)...")...
let average_rating = sqlx::query_scalar("SELECT COALESCE(AVG(r.rating), 0.0)...")...
let wallet_balance = sqlx::query_scalar("SELECT COALESCE(wallet_balance, 0.0)...")...
```

Note: These queries execute sequentially (each `await` blocks). For better performance, they could use `tokio::join!` to run in parallel. This is a potential optimization.

### POST /api/cars/{id}/view (public)

Increments a car's view count. Simple fire-and-forget:
```rust
let _ = sqlx::query("UPDATE cars SET views_count = views_count + 1 WHERE id = $1")
    .bind(car_id)
    .execute(pool.get_ref())
    .await;

HttpResponse::Ok().json(serde_json::json!({"message": "View recorded"}))
```

## 6.8 Upload Handler (src/handlers/upload.rs)

### POST /api/upload (authenticated)

**What it does:** Handles multipart file uploads (images, voice notes).

**Flow:**
```
1. Verify authenticated
2. Read multipart stream
3. Get original filename and extension
4. Validate extension against whitelist
5. Generate unique filename: {user_id}_{uuid}.{ext}
6. Write file to uploads/ directory with 10MB size limit
7. Return URL: /uploads/{filename} or https://api.qent.online/uploads/{filename}
```

**Allowed file types:**
```rust
let allowed = ["jpg", "jpeg", "png", "gif", "webp",   // Images
               "mp3", "m4a", "aac", "ogg", "wav", "opus"]; // Audio (voice notes)
```

**Size limit check:**
```rust
const MAX_SIZE: usize = 10 * 1024 * 1024; // 10MB
while let Some(Ok(chunk)) = field.next().await {
    total_size += chunk.len();
    if total_size > MAX_SIZE {
        let _ = std::fs::remove_file(&filepath);  // Clean up partial file
        return HttpResponse::BadRequest()
            .json(serde_json::json!({"error": "File too large (max 10MB)"}));
    }
    file.write_all(&chunk)?;
}
```

---

# Part 7: WebSocket Implementation

## 7.1 Architecture Overview

The WebSocket system uses Actix's actor model -- each connection is an independent "actor" that processes messages asynchronously.

```
Flutter App (User A)        Rust Backend              Flutter App (User B)
     |                          |                          |
     |--WS connect (JWT)------->|                          |
     |                    ChatWsSession(A)                 |
     |                          |                          |
     |                          |<-----WS connect (JWT)-----|
     |                    ChatWsSession(B)                 |
     |                          |                          |
     |                    +------------+                   |
     |                    | WsManager  |                   |
     |                    +------------+                   |
     |                    |  A -> [Session A addr]         |
     |                    |  B -> [Session B addr]         |
     |                    +------------+                   |
     |                                                     |
     |--chat_message{conv_id,content}->|                   |
     |                     Save to DB                      |
     |                     Find recipient (B)              |
     |                     |--------new_message----------->|
     |<---message_sent-----|                               |
```

## 7.2 The Actor Model

Actix uses the actor model for concurrency. Each actor:
- Has its own state (private data)
- Processes messages one at a time (no race conditions)
- Communicates with other actors via message passing

### WsManager (Central Hub)

```rust
pub struct WsManager {
    sessions: HashMap<Uuid, Vec<Addr<ChatWsSession>>>,
    // Map: user_id -> list of their active WebSocket sessions
    // Vec because a user might have multiple devices connected
}
```

It handles three message types:

**Connect:** When a user opens a WebSocket:
```rust
impl Handler<Connect> for WsManager {
    fn handle(&mut self, msg: Connect, _: &mut Context<Self>) {
        self.sessions.entry(msg.user_id).or_default().push(msg.addr);
    }
}
```

**Disconnect:** When a WebSocket closes:
```rust
impl Handler<Disconnect> for WsManager {
    fn handle(&mut self, msg: Disconnect, _: &mut Context<Self>) {
        if let Some(sessions) = self.sessions.get_mut(&msg.user_id) {
            sessions.retain(|a| a != &msg.addr);  // Remove this session
            if sessions.is_empty() {
                self.sessions.remove(&msg.user_id);  // Clean up
            }
        }
    }
}
```

**SendToUser:** Route a message to a specific user's sessions:
```rust
impl Handler<SendToUser> for WsManager {
    fn handle(&mut self, msg: SendToUser, _: &mut Context<Self>) {
        if let Some(sessions) = self.sessions.get(&msg.user_id) {
            for addr in sessions {
                addr.do_send(WsMessage { ... });  // Send to every device
            }
        }
    }
}
```

### ChatWsSession (Per-Connection)

Each WebSocket connection gets its own `ChatWsSession` actor:

```rust
pub struct ChatWsSession {
    user_id: Uuid,                  // Who this connection belongs to
    hb: Instant,                    // Last heartbeat time
    manager: Addr<WsManager>,       // Address of the central manager
    pool: web::Data<PgPool>,        // Database pool
}
```

**Heartbeat mechanism:**
```rust
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

fn hb(&self, ctx: &mut ws::WebsocketContext<Self>) {
    ctx.run_interval(HEARTBEAT_INTERVAL, |act, ctx| {
        if Instant::now().duration_since(act.hb) > CLIENT_TIMEOUT {
            ctx.stop();  // Kill connection if no response for 30s
            return;
        }
        ctx.ping(b"");  // Send ping every 10s
    });
}
```

Every 10 seconds, the server pings the client. If no pong is received for 30 seconds, the connection is terminated. This detects dropped connections.

## 7.3 Message Types

The WebSocket handles several message types:

### chat_message
Sent by client to deliver a chat message:
```json
{
    "type": "chat_message",
    "conversation_id": "uuid",
    "content": "Hello!",
    "message_type": "text"
}
```

Server flow:
1. Save message to `messages` table
2. Update conversation's `last_message` and `last_message_at`
3. Find the other participant
4. Send `new_message` event to recipient
5. Send `message_sent` confirmation to sender

### typing
Typing indicator:
```json
{
    "type": "typing",
    "conversation_id": "uuid",
    "is_typing": true
}
```

Simply forwarded to the other participant. Not persisted.

### Call Signaling (WebRTC)
For voice/video calls:
```json
{
    "type": "call_offer",      // or call_answer, ice_candidate, call_hangup, call_reject
    "target_id": "user-uuid",
    // ... WebRTC signaling data
}
```

These are forwarded to the target user with the sender's ID added. This enables peer-to-peer calling using WebRTC signaling.

## 7.4 Connection Lifecycle

```
1. Client connects to /ws?token=JWT_TOKEN
2. Server validates JWT from query parameter
3. ws::start() creates ChatWsSession actor
4. Session sends Connect to WsManager
5. Heartbeat ping/pong loop begins
6. Client sends JSON messages
7. Server processes, saves to DB, routes to recipients
8. Client disconnects (or times out)
9. Session sends Disconnect to WsManager
10. WsManager removes session from map
```

---

# Part 8: Payment Integration (Paystack)

## 8.1 Payment Flow Overview

```
    Flutter App              Qent Backend              Paystack
        |                        |                        |
        |-- POST /payments/initiate -->|                  |
        |                        |-- initialize --------->|
        |                        |<-- authorization_url --|
        |<-- {authorization_url} -|                       |
        |                        |                        |
        |-- Open URL in WebView ----->|                   |
        |   (user enters card)   |   (Paystack handles)  |
        |                        |                        |
        |                        |<-- POST webhook -------|
        |                        |   (charge.success)     |
        |                        |-- Verify HMAC sig      |
        |                        |-- Update payment=success
        |                        |-- Update booking=confirmed
        |                        |-- Notify host          |
        |                        |-- Save card (if reusable)
        |                        |-- Send receipt email   |
```

## 8.2 Payment Initiation

```rust
pub async fn initiate_payment(...) -> HttpResponse {
    // 1. Find the booking (must be pending or approved, owned by user)
    let booking = sqlx::query_as::<_, Booking>(
        "SELECT * FROM bookings WHERE id = $1 AND renter_id = $2
         AND status IN ('pending', 'approved')",
    )...;

    // 2. Generate unique reference
    let reference = format!("qent_{}", Uuid::new_v4());

    // 3. Convert to kobo (Paystack uses smallest currency unit)
    let amount_kobo = (booking.total_amount * 100.0) as i64;

    // 4. Call Paystack API
    let paystack_resp = client
        .post("https://api.paystack.co/transaction/initialize")
        .header("Authorization", format!("Bearer {}", config.paystack_secret_key))
        .json(&serde_json::json!({
            "email": email,
            "amount": amount_kobo,
            "reference": reference,
            "currency": "NGN",
        }))
        .send()
        .await;

    // 5. Record payment in database (status: pending)
    sqlx::query("INSERT INTO payments ...")...;

    // 6. Return authorization URL for the client to open
    HttpResponse::Ok().json(PaymentInitResponse {
        authorization_url,   // "https://checkout.paystack.com/xxx"
        reference,
    })
}
```

## 8.3 Webhook Handling (The Most Critical Endpoint)

The webhook is called by Paystack's servers when a payment completes. This is the most security-critical endpoint.

**HMAC Signature Verification:**

```rust
pub async fn paystack_webhook(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    config: web::Data<AppConfig>,
    body_bytes: web::Bytes,        // Raw bytes, not parsed JSON
) -> HttpResponse {
    // 1. Get Paystack's signature from header
    let signature = req.headers()
        .get("x-paystack-signature")
        .and_then(|sig| sig.to_str().ok())
        .unwrap_or("");

    // 2. Compute expected signature using HMAC-SHA512
    let mut mac = Hmac::<Sha512>::new_from_slice(
        config.paystack_secret_key.as_bytes()
    ).expect("HMAC can take key of any size");
    mac.update(&body_bytes);       // Hash the raw request body
    let expected = hex::encode(mac.finalize().into_bytes());

    // 3. Compare signatures
    if signature != expected {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "Invalid signature"}));
    }

    // 4. NOW it's safe to parse and process the webhook
    let body: PaystackWebhookEvent = serde_json::from_slice(&body_bytes)?;
```

**Why raw bytes?** The HMAC must be computed over the exact bytes that were signed. If we parsed to JSON first, re-serializing might change whitespace or key order.

**After verification, on `charge.success`:**
1. Update payment status to `success`
2. Update booking status to `confirmed`
3. Create notification for the host
4. Send booking confirmation email to renter
5. Auto-save card if Paystack says it's reusable

## 8.4 Wallet System

Every user has a `wallet_balance` column. Hosts earn money into their wallet when trips complete.

**Earning flow:**
```
Booking completed -> host gets 85% of subtotal
  |
  v
UPDATE users SET wallet_balance = wallet_balance + $1 WHERE id = host_id
  |
  v
INSERT INTO wallet_transactions (amount=+payout, description="Payout for booking X")
```

**Withdrawal flow:**
```
Host requests withdrawal -> amount <= balance check
  |
  +-- If amount > 100,000: requires admin approval
  |     Debit wallet immediately (hold funds)
  |     Record as "pending_approval"
  |
  +-- If amount <= 100,000: process directly via Paystack
        1. Create transfer recipient (bank account details)
        2. Initiate transfer
        3. Debit wallet
        4. Record wallet transaction (negative amount)
```

```rust
// Create Paystack transfer recipient:
client.post("https://api.paystack.co/transferrecipient")
    .json(&serde_json::json!({
        "type": "nuban",              // Nigerian bank account type
        "account_number": body.account_number,
        "bank_code": body.bank_code,  // e.g., "058" for GTBank
        "currency": "NGN"
    }))

// Initiate transfer:
client.post("https://api.paystack.co/transfer")
    .json(&serde_json::json!({
        "source": "balance",          // From Paystack balance
        "amount": amount_kobo,
        "recipient": recipient_code,
    }))
```

## 8.5 Bank Account Verification

Before withdrawing, users verify their bank account:

```rust
// Resolve account number via Paystack
client.get(&format!(
    "https://api.paystack.co/bank/resolve?account_number={}&bank_code={}",
    body.account_number, body.bank_code
))
```

Returns the account holder's name for the user to confirm.

## 8.6 Bank Listing with Logos

The `list_banks` handler enriches Paystack's bank list with logos from nigerianbanks.xyz:

```rust
// 1. Fetch banks from Paystack
let banks = client.get("https://api.paystack.co/bank?country=nigeria")...;

// 2. Fetch logos from nigerianbanks.xyz
let logos = client.get("https://nigerianbanks.xyz")...;

// 3. Fuzzy match logos to banks by name
let enriched: Vec<serde_json::Value> = banks.into_iter().map(|mut bank| {
    // Try to find a matching logo
    let logo = logo_map.iter().find_map(|(logo_name, url)| {
        if name.contains(logo_name) || logo_name.contains(&name) {
            Some(url.clone())
        } else { None }
    });
    if let Some(logo_url) = logo {
        bank["logo"] = serde_json::Value::String(logo_url);
    }
    bank
}).collect();
```

---

# Part 9: Models & Data Layer

## 9.1 User Model (src/models/user.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::Type, PartialEq)]
#[sqlx(type_name = "user_role", rename_all = "lowercase")]
pub enum UserRole {
    Renter,    // Can book cars
    Host,      // Can list cars + book cars
    Admin,     // Full access
}
```

The `#[sqlx(type_name = "user_role")]` tells SQLx this maps to the PostgreSQL enum `user_role`. The `rename_all = "lowercase"` means `Renter` in Rust becomes `'renter'` in SQL.

**User struct:**
```rust
pub struct User {
    pub id: Uuid,                          // Primary key
    pub email: String,                     // Unique
    pub phone: Option<String>,             // Optional
    pub password_hash: String,             // bcrypt hash (NEVER sent to client)
    pub full_name: String,
    pub role: UserRole,
    pub profile_photo_url: Option<String>, // URL to uploaded photo
    pub drivers_license_url: Option<String>,
    pub id_card_url: Option<String>,
    pub verification_status: VerificationStatus,
    pub wallet_balance: f64,               // Host earnings
    pub is_active: bool,                   // Soft delete flag
    pub country: String,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
```

**UserPublic -- the safe version:**

The `From<User> for UserPublic` trait strips sensitive fields:
```rust
impl From<User> for UserPublic {
    fn from(u: User) -> Self {
        Self {
            id: u.id,
            email: u.email,
            phone: u.phone,
            full_name: u.full_name,
            role: u.role,
            profile_photo_url: u.profile_photo_url,
            verification_status: u.verification_status,
            wallet_balance: u.wallet_balance,
            is_active: u.is_active,
            country: u.country,
            created_at: u.created_at,
            // NOTE: password_hash, drivers_license_url, id_card_url,
            //       updated_at are NOT included
        }
    }
}
```

Usage: `HttpResponse::Ok().json(UserPublic::from(user))` or `user.into()`

**Validation on input types:**
```rust
#[derive(Debug, Deserialize, Validate)]
pub struct SignUpRequest {
    #[validate(email)]           // Must be valid email format
    pub email: String,
    #[validate(length(min = 6))] // At least 6 characters
    pub password: String,
    #[validate(length(min = 2))] // At least 2 characters
    pub full_name: String,
    pub phone: Option<String>,   // Optional, no validation
    pub role: UserRole,
    pub country: Option<String>,
}
```

Used in handlers:
```rust
if let Err(e) = body.validate() {
    return HttpResponse::BadRequest().json(serde_json::json!({"errors": e.to_string()}));
}
```

## 9.2 Car Model (src/models/car.rs)

```rust
pub struct Car {
    // Database columns
    pub id: Uuid,
    pub host_id: Uuid,
    pub make: String,                  // "Toyota"
    pub model: String,                 // "Camry"
    pub year: i32,                     // 2022
    pub color: String,
    pub plate_number: String,
    pub description: String,
    pub price_per_day: f64,
    pub location: String,              // "Lekki, Lagos"
    pub latitude: Option<f64>,         // For distance sorting
    pub longitude: Option<f64>,
    pub photos: Vec<String>,           // Array of photo URLs
    pub features: Vec<String>,         // ["AC", "Bluetooth", "Backup Camera"]
    pub status: CarStatus,
    pub seats: i32,
    pub available_from: Option<NaiveDate>,
    pub available_to: Option<NaiveDate>,
    pub views_count: i32,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,

    // Computed fields (from JOINs, not actual columns)
    pub rating: Option<f64>,           // AVG(reviews.rating) via subquery
    pub trip_count: Option<i64>,       // COUNT(bookings) via subquery
    pub host_name: Option<String>,     // users.full_name via JOIN
}
```

Note that `rating`, `trip_count`, and `host_name` are NOT columns in the `cars` table. They are computed via SQL JOINs. This is why nearly every car query includes this subquery:

```sql
LEFT JOIN (
    SELECT b.car_id,
           AVG(r.rating)::double precision as avg_rating,
           COUNT(DISTINCT b.id) as trip_count
    FROM reviews r
    JOIN bookings b ON r.booking_id = b.id
    GROUP BY b.car_id
) rs ON rs.car_id = c.id
LEFT JOIN users u ON u.id = c.host_id
```

## 9.3 Booking Model (src/models/booking.rs)

```rust
pub struct Booking {
    pub id: Uuid,
    pub car_id: Uuid,
    pub renter_id: Uuid,
    pub host_id: Uuid,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub total_days: i32,
    pub price_per_day: f64,
    pub subtotal: f64,               // price_per_day * total_days
    pub protection_plan_id: Option<Uuid>,
    pub protection_fee: f64,         // plan.daily_rate * total_days
    pub service_fee: f64,            // subtotal * 10%
    pub total_amount: f64,           // subtotal + service_fee + protection_fee
    pub status: BookingStatus,
    pub cancellation_reason: Option<String>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}
```

**BookingWithCar** extends Booking with car and renter info for list views:
```rust
pub struct BookingWithCar {
    // All Booking fields, plus:
    pub car_name: Option<String>,     // "Toyota Camry 2022"
    pub car_photo: Option<String>,    // First photo URL
    pub car_location: Option<String>,
    pub renter_name: Option<String>,
}
```

## 9.4 Payment Model (src/models/payment.rs)

```rust
pub struct Payment {
    pub id: Uuid,
    pub booking_id: Uuid,
    pub payer_id: Uuid,
    pub amount: f64,
    pub currency: String,              // "NGN"
    pub provider: String,              // "paystack"
    pub provider_reference: Option<String>,  // "qent_uuid..."
    pub status: PaymentStatus,         // pending, success, failed, refunded
    pub transaction_type: TransactionType,  // payment, payout, refund
    pub created_at: NaiveDateTime,
}
```

**Paystack webhook types:**
```rust
pub struct PaystackWebhookEvent {
    pub event: String,                 // "charge.success"
    pub data: PaystackWebhookData,
}

pub struct PaystackWebhookData {
    pub reference: String,             // Matches our provider_reference
    pub status: String,
    pub amount: i64,                   // In kobo
    pub currency: String,
    pub authorization: Option<PaystackAuthorization>,  // Card details for saving
}
```

## 9.5 SavedCard Model (src/models/card.rs)

Two versions of the same data:

```rust
// Full version (internal, has authorization_code):
pub struct SavedCard {
    pub authorization_code: String,  // Paystack reuse token (SENSITIVE)
    pub last4: String,               // "1234"
    pub bank: Option<String>,        // "GTBank"
    pub brand: String,               // "Visa"
    pub is_default: bool,
    // ...
}

// Public version (sent to client, NO authorization_code):
pub struct SavedCardPublic {
    pub last4: String,
    pub bank: Option<String>,
    pub brand: String,
    pub is_default: bool,
    // ... (no authorization_code)
}
```

The `From<SavedCard> for SavedCardPublic` conversion strips the authorization code before sending to the client.

## 9.6 Type Mapping: Rust to PostgreSQL

| Rust Type | PostgreSQL Type | Notes |
|-----------|----------------|-------|
| `Uuid` | `UUID` | Primary keys |
| `String` | `VARCHAR(N)` / `TEXT` | Text data |
| `i32` | `INTEGER` | Numbers |
| `i64` | `BIGINT` | Large numbers |
| `f64` | `DOUBLE PRECISION` | Monetary amounts |
| `bool` | `BOOLEAN` | Flags |
| `NaiveDate` | `DATE` | Calendar dates |
| `NaiveDateTime` | `TIMESTAMP` | Date + time |
| `Option<T>` | `T NULL` | Nullable columns |
| `Vec<String>` | `TEXT[]` | PostgreSQL arrays |
| `serde_json::Value` | `JSONB` | JSON data |
| `UserRole` (enum) | `user_role` (enum) | Custom enum type |

---

# Part 10: Error Handling Patterns

## 10.1 The Three Error Patterns in Qent

### Pattern 1: match on Result<Option<T>>

The most common pattern. Used when querying a specific record:

```rust
let result = sqlx::query_as::<_, Car>("SELECT * FROM cars WHERE id = $1")
    .bind(car_id)
    .fetch_optional(pool.get_ref())
    .await;

match result {
    Ok(Some(car)) => HttpResponse::Ok().json(car),
    Ok(None) => HttpResponse::NotFound()
        .json(serde_json::json!({"error": "Car not found"})),
    Err(e) => HttpResponse::InternalServerError()
        .json(serde_json::json!({"error": e.to_string()})),
}
```

### Pattern 2: match then continue

Used when you need the value for further processing:

```rust
let car = match sqlx::query_as::<_, Car>("SELECT * FROM cars WHERE id = $1")
    .bind(car_id)
    .fetch_optional(pool.get_ref())
    .await
{
    Ok(Some(c)) => c,               // Extract the car
    Ok(None) => return HttpResponse::NotFound()
        .json(serde_json::json!({"error": "Car not found"})),
    Err(e) => return HttpResponse::InternalServerError()
        .json(serde_json::json!({"error": e.to_string()})),
};

// Now use `car` for further logic...
if car.host_id == claims.sub {
    return HttpResponse::BadRequest()
        .json(serde_json::json!({"error": "Cannot book your own car"}));
}
```

### Pattern 3: Fire-and-forget with `let _ =`

Used for non-critical operations (notifications, logging):

```rust
// We don't care if the notification fails
let _ = sqlx::query("INSERT INTO notifications ...")
    .bind(...)
    .execute(pool.get_ref())
    .await;
```

The `let _ =` explicitly ignores the Result. Without it, Rust would warn about an unused Result.

## 10.2 Claims Extraction Pattern

Every authenticated handler starts with this:

```rust
let claims = match req.extensions().get::<Claims>().cloned() {
    Some(c) => c,
    None => return HttpResponse::Unauthorized()
        .json(serde_json::json!({"error": "Unauthorized"})),
};
```

This should always succeed (the auth middleware should have set it), but the handler checks anyway for safety.

## 10.3 The `if let` Pattern

A concise match for when you only care about one variant:

```rust
// Instead of:
match existing {
    Ok(true) => {
        return HttpResponse::Conflict()
            .json(serde_json::json!({"error": "Email already registered"}));
    }
    _ => {}  // do nothing for Ok(false) or Err
}

// Use if let:
if let Ok(true) = existing {
    return HttpResponse::Conflict()
        .json(serde_json::json!({"error": "Email already registered"}));
}
```

## 10.4 The .unwrap_or() Pattern

For providing defaults when operations might fail:

```rust
// If query fails, use 0.0 as default:
let balance = sqlx::query_scalar::<_, f64>("SELECT wallet_balance FROM users WHERE id = $1")
    .bind(claims.sub)
    .fetch_one(pool.get_ref())
    .await
    .unwrap_or(0.0);

// If Option is None, use default string:
let country = body.country.clone().unwrap_or_else(|| "Nigeria".to_string());
```

## 10.5 Error Propagation with ?

Used in functions that return Result (mainly middleware):

```rust
pub fn extract_claims(req: &ServiceRequest, jwt_secret: &str) -> Result<Claims, Error> {
    let auth_header = req.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ErrorUnauthorized("Missing authorization header"))?;
        //                                                                 ^ Returns Err if None

    let token = auth_header
        .strip_prefix("Bearer ")
        .ok_or_else(|| ErrorUnauthorized("Invalid authorization format"))?;
        //                                                               ^ Returns Err if no prefix

    let token_data = decode::<Claims>(token, &key, &validation)
        .map_err(|_| ErrorUnauthorized("Invalid token"))?;
        //                                               ^ Returns Err if decode fails

    Ok(token_data.claims)
}
```

Each `?` acts as an early return. If any step fails, the function immediately returns the error.

---

# Part 11: Testing

## 11.1 How to Write Tests for Actix-Web

Actix-Web provides `actix_web::test` utilities. Here is how you would test a handler:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{test, App};

    #[actix_web::test]
    async fn test_health_check() {
        // Create test app
        let app = test::init_service(
            App::new().route("/health", web::get().to(handlers::health::health_check))
        ).await;

        // Make test request
        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        // Assert response
        assert_eq!(resp.status(), 200);
    }
}
```

## 11.2 Testing with Database

For integration tests that need a database:

```rust
#[actix_web::test]
async fn test_sign_up() {
    // Set up test database pool
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();

    // Run migrations
    sqlx::migrate!("./migrations").run(&pool).await.unwrap();

    // Create test app with real pool
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(AppConfig::from_env()))
            .route("/api/auth/signup", web::post().to(handlers::auth::sign_up))
    ).await;

    // Make request
    let req = test::TestRequest::post()
        .uri("/api/auth/signup")
        .set_json(&serde_json::json!({
            "email": "test@example.com",
            "password": "secret123",
            "full_name": "Test User",
            "role": "renter"
        }))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 201);

    // Clean up
    sqlx::query("DELETE FROM users WHERE email = 'test@example.com'")
        .execute(&pool)
        .await
        .unwrap();
}
```

## 11.3 Testing Authenticated Routes

```rust
#[actix_web::test]
async fn test_get_profile() {
    let pool = /* ... */;
    let config = AppConfig::from_env();

    // Generate a test JWT
    let claims = Claims {
        sub: test_user_id,
        role: UserRole::Renter,
        exp: (Utc::now() + Duration::hours(1)).timestamp() as usize,
    };
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(config.jwt_secret.as_bytes()),
    ).unwrap();

    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(pool.clone()))
            .app_data(web::Data::new(config.clone()))
            .service(
                web::scope("")
                    .wrap(actix_web::middleware::from_fn(auth_mw))
                    .route("/profile", web::get().to(handlers::auth::get_profile))
            )
    ).await;

    let req = test::TestRequest::get()
        .uri("/profile")
        .insert_header(("Authorization", format!("Bearer {}", token)))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 200);
}
```

---

# Part 12: Deployment & Configuration

## 12.1 Environment Variables

The `AppConfig` struct in `src/services/mod.rs` centralizes all configuration:

```rust
pub struct AppConfig {
    pub database_url: String,         // PostgreSQL connection string
    pub jwt_secret: String,           // Secret for signing JWTs
    pub paystack_secret_key: String,  // Paystack API key
    pub resend_api_key: String,       // Resend email API key
    pub app_url: String,              // Base URL (e.g., "https://api.qent.online")
    pub host: String,                 // Bind address (e.g., "0.0.0.0")
    pub port: u16,                    // Bind port (e.g., 8080)
}
```

**Required variables** (app crashes without them):
- `DATABASE_URL` -- e.g., `postgres://user:pass@host:5432/qent`
- `JWT_SECRET` -- any long random string
- `PAYSTACK_SECRET_KEY` -- from Paystack dashboard

**Optional variables** (have defaults):
- `RESEND_API_KEY` -- defaults to empty (emails skipped)
- `APP_URL` -- defaults to `http://localhost:8080`
- `HOST` -- defaults to `127.0.0.1`
- `PORT` -- defaults to `8080`

### Loading config:

```rust
impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL")
                .expect("DATABASE_URL must be set"),  // Crash if missing
            jwt_secret: std::env::var("JWT_SECRET")
                .expect("JWT_SECRET must be set"),
            resend_api_key: std::env::var("RESEND_API_KEY")
                .unwrap_or_default(),                  // Default to "" if missing
            host: std::env::var("HOST")
                .unwrap_or_else(|_| "127.0.0.1".to_string()),
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()
                .unwrap_or(8080),                     // Parse string to u16
        }
    }
}
```

### .env file:

```bash
DATABASE_URL=postgres://qent:password@localhost:5432/qent
JWT_SECRET=your-super-secret-jwt-key-here
PAYSTACK_SECRET_KEY=sk_test_xxxxxxxxxxxxx
RESEND_API_KEY=re_xxxxxxxxxxxxx
APP_URL=http://localhost:8080
HOST=127.0.0.1
PORT=8080
RUST_LOG=info   # Logging level (not in AppConfig, used by env_logger)
```

`dotenv::dotenv().ok()` in main.rs loads this file. The `.ok()` means it is fine if the file does not exist (in production, env vars are set directly).

## 12.2 Database Connection

```rust
let pool = PgPoolOptions::new()
    .max_connections(10)               // 10 connections for all handlers
    .connect(&config.database_url)     // Connection string from env
    .await
    .expect("Failed to create database pool");
```

In production (e.g., Render), the `DATABASE_URL` might look like:
```
postgres://qent_user:randompassword@dpg-xxxxx.oregon-postgres.render.com:5432/qent_db
```

With `tls-rustls` feature enabled in SQLx, the connection is encrypted.

## 12.3 Background Tasks

```rust
// Clone pool because tokio::spawn takes ownership
let bg_pool = pool.clone();
tokio::spawn(auto_complete_bookings(bg_pool));
```

`tokio::spawn` runs a future on the tokio runtime without blocking the main thread. The `auto_complete_bookings` function runs an infinite loop:

```rust
async fn auto_complete_bookings(pool: PgPool) {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));
    loop {
        interval.tick().await;  // Wait 1 hour
        // Find overdue active bookings and complete them
        // Credit host wallets
        // Send notifications
    }
}
```

This runs alongside the web server, checking every hour for bookings that should be auto-completed.

## 12.4 CORS for Production

For production, you need to include all domains that will access your API:
- Your landing page domain
- Your admin dashboard domain
- Your API domain (for same-origin requests)
- Flutter app (does not need CORS, it makes direct HTTP requests)

---

# Part 13: Common Patterns Reference

## 13.1 How to Add a New Endpoint

**Step 1: Create the request/response types (if needed)**

In `src/models/` (new file or existing):
```rust
// src/models/example.rs
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Serialize, FromRow)]
pub struct MyEntity {
    pub id: Uuid,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMyEntityRequest {
    pub name: String,
}
```

Register it in `src/models/mod.rs`:
```rust
pub mod example;
pub use example::*;
```

**Step 2: Create the handler**

In `src/handlers/` (new file or existing):
```rust
// src/handlers/example.rs
use actix_web::{web, HttpMessage, HttpRequest, HttpResponse};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{Claims, MyEntity, CreateMyEntityRequest};

pub async fn create_entity(
    req: HttpRequest,
    pool: web::Data<PgPool>,
    body: web::Json<CreateMyEntityRequest>,
) -> HttpResponse {
    let claims = match req.extensions().get::<Claims>().cloned() {
        Some(c) => c,
        None => return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "Unauthorized"})),
    };

    let id = Uuid::new_v4();
    let result = sqlx::query_as::<_, MyEntity>(
        "INSERT INTO my_entities (id, name, user_id) VALUES ($1, $2, $3) RETURNING *"
    )
    .bind(id)
    .bind(&body.name)
    .bind(claims.sub)
    .fetch_one(pool.get_ref())
    .await;

    match result {
        Ok(entity) => HttpResponse::Created().json(entity),
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": e.to_string()})),
    }
}
```

Register it in `src/handlers/mod.rs`:
```rust
pub mod example;
```

**Step 3: Add the route**

In `src/main.rs`, inside the appropriate scope:
```rust
// For authenticated routes:
.route("/entities", web::post().to(handlers::example::create_entity))

// For public routes:
// (add outside the authenticated scope)
```

## 13.2 How to Add a New Model/Table

**Step 1: Create migration**

Create `migrations/017_my_new_table.sql`:
```sql
CREATE TABLE my_entities (
    id UUID PRIMARY KEY,
    user_id UUID NOT NULL REFERENCES users(id),
    name VARCHAR(255) NOT NULL,
    created_at TIMESTAMP NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_my_entities_user ON my_entities(user_id);
```

**Step 2: Create Rust model** (as shown in 13.1)

**Step 3:** Restart the server. Migrations run automatically.

## 13.3 How to Add a New Migration

1. Create a new file in `migrations/` with the next number:
   ```
   migrations/017_descriptive_name.sql
   ```
2. Write your SQL (CREATE TABLE, ALTER TABLE, CREATE INDEX, etc.)
3. Restart the server or run `sqlx migrate run`

**Important:** Never modify existing migration files. If you need to change a table, create a NEW migration with ALTER TABLE.

## 13.4 How to Add Middleware to a Route

**Option A: Wrap an entire scope**
```rust
.service(
    web::scope("/admin")
        .wrap(actix_web::middleware::from_fn(admin_only_mw))
        .route("/users", web::get().to(handlers::admin::list_users))
)
```

**Option B: Check inside the handler**
```rust
pub async fn admin_action(req: HttpRequest) -> HttpResponse {
    let claims = req.extensions().get::<Claims>().cloned().unwrap();
    if claims.role != UserRole::Admin {
        return HttpResponse::Forbidden()
            .json(serde_json::json!({"error": "Admin only"}));
    }
    // ... continue
}
```

Option B is simpler and is what Qent uses for admin routes (they are inside the authenticated scope, and each handler checks `claims.role`).

## 13.5 Common SQL Patterns Used in Qent

### COALESCE for partial updates:
```sql
UPDATE users SET
    full_name = COALESCE($1, full_name),  -- Use new value, or keep old
    phone = COALESCE($2, phone)
WHERE id = $3
```

### EXISTS for boolean checks:
```sql
SELECT EXISTS(SELECT 1 FROM users WHERE email = $1)
-- Returns true/false, fast because it stops at first match
```

### CTE with RETURNING for insert-then-join:
```sql
WITH inserted AS (
    INSERT INTO cars (...) VALUES (...) RETURNING *
)
SELECT inserted.*, u.full_name as host_name
FROM inserted
LEFT JOIN users u ON u.id = inserted.host_id
```

### Array access for first photo:
```sql
c.photos[1] as car_photo  -- PostgreSQL arrays are 1-indexed
```

### String concatenation:
```sql
c.make || ' ' || c.model || ' ' || c.year::text AS car_name
```

### Conditional aggregation:
```sql
SELECT
    COALESCE(SUM(CASE WHEN status = 'completed' THEN subtotal * 0.85 ELSE 0 END), 0) as total_earned,
    COUNT(CASE WHEN status = 'completed' THEN 1 END) as completed_trips
FROM bookings
WHERE host_id = $1
```

### Nullable parameter filtering:
```sql
WHERE ($1::text IS NULL OR LOWER(c.location) LIKE LOWER('%' || $1 || '%'))
```

## 13.6 Quick Debugging Reference

**Enable detailed logging:**
```bash
RUST_LOG=debug cargo run
# Or for specific modules:
RUST_LOG=qent=debug,sqlx=info cargo run
```

**Common compile errors and what they mean:**

| Error | Meaning | Fix |
|-------|---------|-----|
| "value moved here" | You used a value after it was moved | Add `.clone()` or use a reference `&` |
| "borrowed value does not live long enough" | A reference outlives its source | Own the data instead of borrowing |
| "cannot borrow as mutable" | Trying to mutate a shared reference | Use `&mut` or interior mutability |
| "the trait `FromRow` is not implemented" | Struct doesn't derive FromRow | Add `#[derive(FromRow)]` |
| "the trait `Serialize` is not implemented" | Can't convert to JSON | Add `#[derive(Serialize)]` |
| "mismatched types: Option vs T" | Database column might be NULL | Use `Option<T>` for nullable columns |
| "column not found" | SQL column name doesn't match struct field | Check column aliases in your SQL |

## 13.7 The Full Request Pipeline Diagram

```
  Client sends:  POST /api/bookings
                 Authorization: Bearer eyJhbGci...
                 Content-Type: application/json
                 {"car_id": "...", "start_date": "2026-04-01", "end_date": "2026-04-05"}
                           |
                           v
  +----------------------------------------------------------------------+
  | 1. TCP Connection (tokio/actix runtime)                              |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 2. CORS Middleware                                                    |
  |    - Check Origin header against allowed list                        |
  |    - Add Access-Control-Allow-Origin header to response              |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 3. Logger Middleware                                                  |
  |    - Log: "POST /api/bookings" with timestamp                        |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 4. Route Matching                                                    |
  |    - Match /api -> scope                                             |
  |    - /bookings matches authenticated scope route                     |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 5. auth_mw (JWT Middleware)                                          |
  |    - Extract "Bearer eyJhbGci..." from Authorization header          |
  |    - Decode JWT using JWT_SECRET                                     |
  |    - Validate expiration                                             |
  |    - Store Claims{sub: uuid, role: renter} in request extensions     |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 6. Extractor: web::Json<CreateBookingRequest>                        |
  |    - Read request body bytes                                         |
  |    - Deserialize JSON into CreateBookingRequest struct                |
  |    - If malformed, return 400 Bad Request                            |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 7. Handler: create_booking()                                         |
  |    a. Extract Claims from extensions                                 |
  |    b. Query car from database (verify active)                        |
  |    c. Check not self-booking                                         |
  |    d. Check date overlap                                             |
  |    e. Calculate pricing                                              |
  |    f. Insert booking row                                             |
  |    g. Create notification for host                                   |
  |    h. Return HttpResponse::Created().json(booking)                   |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 8. Serialization                                                     |
  |    - Booking struct -> JSON via serde                                 |
  |    - Set Content-Type: application/json                              |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 9. Logger Middleware (after)                                          |
  |    - Log: "POST /api/bookings -> 201 (15ms)"                        |
  +----------------------------------------------------------------------+
                           |
                           v
  +----------------------------------------------------------------------+
  | 10. CORS Middleware (after)                                           |
  |     - Add CORS headers to response                                   |
  +----------------------------------------------------------------------+
                           |
                           v
  Client receives: 201 Created
                   {"id": "...", "car_id": "...", "status": "pending", ...}
```

---

## Appendix A: Glossary

| Term | Definition |
|------|-----------|
| **Actor** | An independent concurrent unit that processes messages sequentially (Actix pattern) |
| **bcrypt** | Password hashing algorithm designed to be intentionally slow |
| **Claims** | The data payload inside a JWT (user ID, role, expiration) |
| **CTE** | Common Table Expression -- a temporary result set in SQL (`WITH ... AS`) |
| **Derive macro** | Automatically implements a trait for a struct/enum (`#[derive(Debug)]`) |
| **Extractor** | Actix-Web's system for parsing request data into typed parameters |
| **FromRow** | SQLx trait that maps database rows to Rust structs |
| **Future** | Rust's representation of an async operation (like a JS Promise) |
| **HMAC** | Hash-based Message Authentication Code -- verifies message integrity |
| **JWT** | JSON Web Token -- a signed, base64-encoded JSON object for auth |
| **Kobo** | Smallest unit of Nigerian Naira (1 NGN = 100 kobo), used by Paystack |
| **Migration** | A SQL file that modifies the database schema |
| **NDPA** | Nigeria Data Protection Act -- data privacy law |
| **NUBAN** | Nigerian Uniform Bank Account Number standard |
| **PgPool** | A pool of PostgreSQL connections for concurrent use |
| **Scope** | An Actix-Web route prefix that can have its own middleware |
| **Serde** | Rust's serialization/deserialization framework |
| **Token bucket** | Rate limiting algorithm used by actix-governor |
| **Trait** | Rust's version of an interface -- defines shared behavior |
| **WebSocket** | Persistent bidirectional connection for real-time messaging |

## Appendix B: All API Endpoints Quick Reference

| Method | Path | Auth? | Description |
|--------|------|-------|-------------|
| GET | /health | No | Health check |
| GET | /ws?token=JWT | Token in query | WebSocket connection |
| POST | /api/auth/signup | No (rate limited) | Create account |
| POST | /api/auth/signin | No (rate limited) | Sign in |
| POST | /api/auth/refresh | No (rate limited) | Refresh JWT |
| POST | /api/auth/forgot-password | No (rate limited) | Request password reset |
| POST | /api/auth/reset-password | No (rate limited) | Reset password with code |
| POST | /api/auth/send-code | No (rate limited) | Send verification code |
| POST | /api/auth/verify-code | No (rate limited) | Verify email code |
| GET | /api/cars/search | No | Search cars with filters |
| GET | /api/cars/homepage | No | Homepage feed sections |
| GET | /api/cars/{id} | No | Get car details |
| POST | /api/cars/{id}/view | No | Increment view count |
| GET | /api/protection-plans | No | List insurance plans |
| GET | /api/users/{id} | No | Get public profile |
| GET | /api/users/{id}/reviews | No | Get user reviews |
| GET | /api/users/{id}/rating | No | Get user rating |
| GET | /api/payments/banks | No | List Nigerian banks |
| POST | /api/payments/verify-account | No | Verify bank account |
| POST | /api/payments/webhook | No (HMAC) | Paystack webhook |
| POST | /api/waitlist | No | Join waitlist |
| GET | /api/waitlist/count | No | Get waitlist count |
| GET | /api/profile | Yes | Get own profile |
| PUT | /api/profile | Yes | Update profile |
| POST | /api/profile/verify-identity | Yes | Submit ID documents |
| POST | /api/cars | Yes (Host) | Create car listing |
| GET | /api/cars/my-listings | Yes | Get own listings |
| PUT | /api/cars/{id} | Yes (Owner) | Update car |
| POST | /api/cars/{id}/deactivate | Yes (Owner) | Deactivate car |
| GET | /api/cars/{id}/booked-dates | Yes | Get booked date ranges |
| GET | /api/dashboard/stats | Yes | Host dashboard stats |
| GET | /api/dashboard/listings | Yes | Host listing stats |
| POST | /api/bookings | Yes | Create booking |
| GET | /api/bookings/mine | Yes | Get my bookings |
| GET | /api/bookings/{id} | Yes | Get booking details |
| POST | /api/bookings/{id}/action | Yes | Change booking status |
| GET | /api/bookings/host/pending | Yes | Get pending bookings for host |
| GET | /api/payments/wallet | Yes | Get wallet balance |
| GET | /api/payments/wallet/transactions | Yes | Get transaction history |
| GET | /api/payments/earnings | Yes | Get earnings breakdown |
| POST | /api/payments/initiate | Yes | Start payment |
| POST | /api/payments/withdraw | Yes | Withdraw to bank |
| POST | /api/payments/refund/{id} | Yes | Request refund |
| GET | /api/cards | Yes | List saved cards |
| POST | /api/cards/{id}/default | Yes | Set default card |
| DELETE | /api/cards/{id} | Yes | Delete saved card |
| POST | /api/cards/charge | Yes | Charge saved card |
| POST | /api/reviews | Yes | Create review |
| GET | /api/favorites | Yes | Get favorite cars |
| POST | /api/favorites/{id} | Yes | Toggle favorite |
| GET | /api/favorites/{id}/check | Yes | Check if favorited |
| GET | /api/notifications | Yes | Get notifications |
| POST | /api/notifications/{id}/read | Yes | Mark notification read |
| POST | /api/notifications/read-all | Yes | Mark all read |
| POST | /api/partner/apply | Yes | Apply to be partner |
| GET | /api/partner/application | Yes | Get application status |
| GET | /api/partner/dashboard | Yes | Partner dashboard |
| POST | /api/partner/activate-car | Yes | Activate partner car |
| GET | /api/stories | Yes | Get host stories |
| POST | /api/stories | Yes | Create story |
| DELETE | /api/stories/{id} | Yes | Delete story |
| POST | /api/chat/conversations | Yes | Get/create conversation |
| GET | /api/chat/conversations | Yes | List conversations |
| GET | /api/chat/conversations/{id}/messages | Yes | Get messages |
| POST | /api/chat/conversations/{id}/messages | Yes | Send message |
| POST | /api/chat/conversations/{id}/read | Yes | Mark conversation read |
| DELETE | /api/chat/conversations/{id} | Yes | Delete conversation |
| POST | /api/upload | Yes | Upload file |
| POST | /api/auth/accept-terms | Yes | Accept terms of service |
| GET | /api/auth/terms-status | Yes | Check terms status |
| POST | /api/account/request-deletion | Yes | Request account deletion |
| POST | /api/account/cancel-deletion | Yes | Cancel deletion request |
| GET | /api/account/export | Yes | Export personal data |
| POST | /api/damage-reports | Yes | Create damage report |
| GET | /api/damage-reports/{id} | Yes | Get damage reports |
| GET | /api/admin/users | Yes (Admin) | List all users |
| POST | /api/admin/users/{id}/verify | Yes (Admin) | Verify user |
| POST | /api/admin/users/{id}/reject | Yes (Admin) | Reject verification |
| POST | /api/admin/users/{id}/deactivate | Yes (Admin) | Deactivate user |
| GET | /api/admin/cars | Yes (Admin) | List all cars |
| POST | /api/admin/cars/{id}/approve | Yes (Admin) | Approve car listing |
| POST | /api/admin/cars/{id}/reject | Yes (Admin) | Reject car listing |
| GET | /api/admin/bookings | Yes (Admin) | List all bookings |
| POST | /api/admin/bookings/{id}/dispute-refund | Yes (Admin) | Handle dispute |
| GET | /api/admin/payments | Yes (Admin) | List all payments |
| GET | /api/admin/analytics | Yes (Admin) | Get platform analytics |
| GET | /api/admin/audit-log | Yes (Admin) | View audit log |
| GET | /api/admin/withdrawals/pending | Yes (Admin) | List pending withdrawals |
| POST | /api/admin/withdrawals/{id}/approve | Yes (Admin) | Approve withdrawal |
| POST | /api/admin/withdrawals/{id}/reject | Yes (Admin) | Reject withdrawal |

---

*This guide covers every file in the Qent backend as of March 2026. As you add features, refer to Part 13 for patterns to follow. When in doubt, find a similar existing handler and follow its structure.*
