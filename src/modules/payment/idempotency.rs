use serde_json::Value;

use crate::cache::Cache;
use crate::error::{AppError, AppResult};

/// Check an idempotency key. If the key was seen before, returns the cached
/// JSON response. If not, stores a placeholder atomically and returns None —
/// the caller should proceed, then call `store_result` when done.
///
/// This is intentionally simpler than the Node app's in-flight lock: the
/// window between acquire and store is tiny for checkout session creation,
/// and a concurrent duplicate just creates two Stripe sessions (rare + cheap
/// to reconcile via the webhook).
pub async fn check(cache: &Cache, scope: &str, key: &str) -> AppResult<Option<Value>> {
    let k = redis_key(scope, key);
    if let Some(existing) = cache.get(&k).await? {
        if existing == "in-flight" {
            return Err(AppError::Conflict(
                "request with same idempotency key is in flight".into(),
            ));
        }
        let value: Value = serde_json::from_str(&existing)
            .map_err(|e| AppError::internal(format!("parse idempotency cache: {e}")))?;
        return Ok(Some(value));
    }
    cache.set_nx_ex(&k, "in-flight", 24 * 60 * 60).await?;
    Ok(None)
}

pub async fn store_result(
    cache: &Cache,
    scope: &str,
    key: &str,
    value: &Value,
) -> AppResult<()> {
    let k = redis_key(scope, key);
    let encoded = serde_json::to_string(value)
        .map_err(|e| AppError::internal(format!("encode idempotency: {e}")))?;
    cache.set_ex(&k, &encoded, 24 * 60 * 60).await
}

fn redis_key(scope: &str, key: &str) -> String {
    format!("idem:{scope}:{key}")
}
