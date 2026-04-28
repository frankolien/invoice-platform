//! Argon2 password hashing.
//!
//! Hashing/verification is CPU-bound (~100-300ms with default params on
//! commodity hardware — that cost is the point, it makes brute-force
//! infeasible). If we ran it on the async worker thread, every concurrent
//! login or register would block one of the small pool of Tokio worker
//! threads, tanking throughput. So both functions hop to the blocking
//! thread pool via `spawn_blocking`.
//!
//! We also cap *in-flight* hashes with a semaphore sized to CPU count.
//! Argon2id with default params allocates ~19 MiB per call. Without the
//! cap, a burst of N concurrent registers all spawn_blocking-allocate
//! ~19 MiB up front before the blocking pool can drain them — at 50 VUs
//! that's ~1 GiB instantly, and the container OOMs under sustained load.
//! With the cap, memory is bounded to roughly `cpus × 19 MiB` regardless
//! of request rate; throughput stabilizes at `cpus / hash_time` instead
//! of crashing.

use std::sync::OnceLock;

use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString, rand_core::OsRng},
};
use tokio::sync::Semaphore;

use crate::error::{AppError, AppResult};

fn hash_semaphore() -> &'static Semaphore {
    static SEM: OnceLock<Semaphore> = OnceLock::new();
    SEM.get_or_init(|| {
        let permits = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);
        Semaphore::new(permits)
    })
}

pub async fn hash(password: &str) -> AppResult<String> {
    let _permit = hash_semaphore()
        .acquire()
        .await
        .map_err(|e| AppError::internal(format!("hash sem: {e}")))?;
    let password = password.to_owned();
    tokio::task::spawn_blocking(move || hash_sync(&password))
        .await
        .map_err(|e| AppError::internal(format!("hash join: {e}")))?
}

pub async fn verify(password: &str, hash: &str) -> AppResult<bool> {
    let _permit = hash_semaphore()
        .acquire()
        .await
        .map_err(|e| AppError::internal(format!("verify sem: {e}")))?;
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
