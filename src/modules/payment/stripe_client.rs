use stripe::Client;

use crate::error::{AppError, AppResult};

/// Thin wrapper around the Stripe client. Held in app state as
/// `Option<StripeClient>` so the service can boot without Stripe credentials
/// (dev + tests). Endpoints that need Stripe return 503 when it's unset.
#[derive(Clone)]
pub struct StripeClient {
    pub client: Client,
    pub webhook_secret: String,
}

impl StripeClient {
    pub fn new(secret_key: &str, webhook_secret: &str) -> Self {
        Self {
            client: Client::new(secret_key),
            webhook_secret: webhook_secret.to_string(),
        }
    }
}

pub fn require_stripe(
    client: Option<&actix_web::web::Data<StripeClient>>,
) -> AppResult<&StripeClient> {
    client
        .map(|c| c.get_ref())
        .ok_or_else(|| AppError::BadRequest("stripe is not configured".into()))
}
