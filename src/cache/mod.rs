use redis::aio::ConnectionManager;

use crate::error::{AppError, AppResult};

#[derive(Clone)]
pub struct Cache {
    conn: ConnectionManager,
}

impl Cache {
    pub async fn connect(redis_url: &str) -> anyhow::Result<Self> {
        let client = redis::Client::open(redis_url)?;
        let conn = ConnectionManager::new(client).await?;
        Ok(Self { conn })
    }

    /// Atomically sets `key` to `value` only if the key does not already exist.
    /// Returns `true` if the key was set, `false` if it already existed.
    /// TTL is applied on set (so we don't end up with a key without expiry if
    /// a later EXPIRE call fails).
    pub async fn set_nx_ex(&self, key: &str, value: &str, ttl_secs: u64) -> AppResult<bool> {
        let mut conn = self.conn.clone();
        let set: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(value)
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis SET NX EX: {e}")))?;
        Ok(set.is_some())
    }

    pub async fn get(&self, key: &str) -> AppResult<Option<String>> {
        let mut conn = self.conn.clone();
        redis::cmd("GET")
            .arg(key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis GET: {e}")))
    }

    pub async fn set_ex(&self, key: &str, value: &str, ttl_secs: u64) -> AppResult<()> {
        let mut conn = self.conn.clone();
        redis::cmd("SET")
            .arg(key)
            .arg(value)
            .arg("EX")
            .arg(ttl_secs)
            .query_async::<()>(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis SET EX: {e}")))
    }

    /// INCR a counter and set TTL on first increment only.
    /// Returns the post-increment value.
    pub async fn incr_with_ttl(&self, key: &str, ttl_secs: u64) -> AppResult<u64> {
        let mut conn = self.conn.clone();
        let (count,): (u64,) = redis::pipe()
            .atomic()
            .cmd("INCR")
            .arg(key)
            .cmd("EXPIRE")
            .arg(key)
            .arg(ttl_secs)
            .arg("NX") // only set TTL if not already set
            .ignore()
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis INCR+EXPIRE: {e}")))?;
        Ok(count)
    }

    pub async fn ttl(&self, key: &str) -> AppResult<i64> {
        let mut conn = self.conn.clone();
        redis::cmd("TTL")
            .arg(key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis TTL: {e}")))
    }
}
