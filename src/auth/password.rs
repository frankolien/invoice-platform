//! Argon2 password hashing.
//!
//! Hashing/verification is CPU-bound (~100-300ms with default params on
//! commodity hardware — that cost is the point, it makes brute-force
//! infeasible). If we ran it on the async worker thread, every concurrent
//! login or register would block one of the small pool of Tokio worker
//! threads, tanking throughput. So both functions hop to the blocking
//! thread pool via `spawn_blocking`.

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};

use crate::error::{AppError, AppResult};

pub async fn hash(password: &str) -> AppResult<String> {
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_sync(&password))
        .await
        .map_err(|e| AppError::internal(format!("hash join: {e}")))?
}

pub async fn verify(password: &str, hash: &str) -> AppResult<bool> {
    let password = password.to_owned();
    let hash = hash.to_owned();
    tokio::task::spawn_blocking(move || verify_sync(&password, &hash))
        .await
        .map_err(|e| AppError::internal(format!("verify join: {e}")))?
}

fn hash_sync(password: &str) -> AppResult<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::internal(format!("hash failure: {e}")))
}

fn verify_sync(password: &str, hash: &str) -> AppResult<bool> {
    let parsed = PasswordHash::new(hash)
        .map_err(|e| AppError::internal(format!("invalid hash: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}
