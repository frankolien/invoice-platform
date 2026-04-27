//! A simple three-state circuit breaker for wrapping external service calls.
//!
//! States:
//!   - **Closed**: requests pass through. Consecutive failures are counted.
//!     When the count reaches `failure_threshold`, transition to Open.
//!   - **Open**: every request is rejected instantly with `BreakerOpen`,
//!     no underlying call is made. After `cooldown`, transition to HalfOpen.
//!   - **HalfOpen**: probe state. Requests pass through; consecutive
//!     successes count toward `success_threshold` to close. Any failure
//!     re-opens immediately.
//!
//! Why this matters: calling Stripe (or any external service) when it's
//! down means your worker pool fills with requests stuck in 30s timeouts.
//! Better to fail fast and retry once the service is back.
//!
//! This implementation is intentionally minimal. No half-open concurrency
//! limit, no rolling window, no per-error-type policy. If we need any of
//! those, we add them.

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Closed,
    Open,
    HalfOpen,
}

impl State {
    /// Numeric encoding for Prometheus: 0=closed, 1=open, 2=half-open.
    pub fn as_metric(&self) -> i64 {
        match self {
            State::Closed => 0,
            State::Open => 1,
            State::HalfOpen => 2,
        }
    }
}

#[derive(Debug, Error)]
pub enum BreakerError<E: fmt::Debug> {
    #[error("circuit breaker open")]
    Open,
    #[error("inner error: {0:?}")]
    Inner(E),
}

#[derive(Clone)]
pub struct CircuitBreakerConfig {
    pub name: &'static str,
    pub failure_threshold: u32,
    pub cooldown: Duration,
    pub success_threshold: u32,
}

impl CircuitBreakerConfig {
    pub const STRIPE: Self = Self {
        name: "stripe",
        failure_threshold: 5,
        cooldown: Duration::from_secs(30),
        success_threshold: 2,
    };
    pub const EMAIL: Self = Self {
        name: "email",
        failure_threshold: 3,
        cooldown: Duration::from_secs(60),
        success_threshold: 1,
    };
    pub const WEBHOOK: Self = Self {
        name: "webhook",
        failure_threshold: 10,
        cooldown: Duration::from_secs(15),
        success_threshold: 3,
    };
}

struct Inner {
    state: State,
    failures: u32,
    successes: u32,
    opened_at: Option<Instant>,
}

#[derive(Clone)]
pub struct CircuitBreaker {
    cfg: CircuitBreakerConfig,
    inner: Arc<Mutex<Inner>>,
}

impl CircuitBreaker {
    pub fn new(cfg: CircuitBreakerConfig) -> Self {
        Self {
            cfg,
            inner: Arc::new(Mutex::new(Inner {
                state: State::Closed,
                failures: 0,
                successes: 0,
                opened_at: None,
            })),
        }
    }

    pub fn name(&self) -> &'static str {
        self.cfg.name
    }

    pub fn state(&self) -> State {
        self.inner.lock().state
    }

    /// Run `f` if the breaker permits. On Open, returns `BreakerError::Open`
    /// without calling. On Closed/HalfOpen, calls and updates state from
    /// the result.
    pub async fn call<F, Fut, T, E>(&self, f: F) -> Result<T, BreakerError<E>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, E>>,
        E: fmt::Debug,
    {
        // Pre-call: maybe transition Open -> HalfOpen if cooldown elapsed.
        {
            let mut inner = self.inner.lock();
            if inner.state == State::Open {
                let elapsed = inner.opened_at.map(|t| t.elapsed()).unwrap_or_default();
                if elapsed >= self.cfg.cooldown {
                    tracing::info!(breaker = self.cfg.name, "transition open -> half-open");
                    inner.state = State::HalfOpen;
                    inner.successes = 0;
                    inner.failures = 0;
                } else {
                    return Err(BreakerError::Open);
                }
            }
        }

        let result = f().await;

        // Post-call: count + maybe transition.
        let mut inner = self.inner.lock();
        match &result {
            Ok(_) => {
                inner.failures = 0;
                if inner.state == State::HalfOpen {
                    inner.successes += 1;
                    if inner.successes >= self.cfg.success_threshold {
                        tracing::info!(
                            breaker = self.cfg.name,
                            "transition half-open -> closed"
                        );
                        inner.state = State::Closed;
                        inner.successes = 0;
                        inner.opened_at = None;
                    }
                }
            }
            Err(_) => {
                inner.successes = 0;
                inner.failures += 1;

                let should_open = match inner.state {
                    State::HalfOpen => true,
                    State::Closed => inner.failures >= self.cfg.failure_threshold,
                    State::Open => false, // unreachable: pre-call returned
                };
                if should_open {
                    tracing::warn!(
                        breaker = self.cfg.name,
                        failures = inner.failures,
                        "transition -> open"
                    );
                    inner.state = State::Open;
                    inner.opened_at = Some(Instant::now());
                }
            }
        }

        result.map_err(BreakerError::Inner)
    }
}

/// Bundle of pre-configured breakers for the three external services we
/// talk to. Cloned into app state once at startup.
#[derive(Clone)]
pub struct CircuitBreakers {
    pub stripe: CircuitBreaker,
    pub email: CircuitBreaker,
    pub webhook: CircuitBreaker,
}

impl CircuitBreakers {
    pub fn new() -> Self {
        Self {
            stripe: CircuitBreaker::new(CircuitBreakerConfig::STRIPE),
            email: CircuitBreaker::new(CircuitBreakerConfig::EMAIL),
            webhook: CircuitBreaker::new(CircuitBreakerConfig::WEBHOOK),
        }
    }

    /// Snapshots the three breaker states into the Prometheus gauge.
    /// Call from a periodic task or on every /metrics scrape; cheap.
    pub fn export_metrics(&self, m: &crate::observability::metrics::Metrics) {
        for b in [&self.stripe, &self.email, &self.webhook] {
            m.circuit_breaker_state
                .with_label_values(&[b.name()])
                .set(b.state().as_metric());
        }
    }
}

impl Default for CircuitBreakers {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn opens_after_threshold_failures() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            name: "test",
            failure_threshold: 2,
            cooldown: Duration::from_millis(50),
            success_threshold: 1,
        });

        async fn fail() -> Result<(), &'static str> {
            Err("boom")
        }

        let _ = cb.call(fail).await;
        assert_eq!(cb.state(), State::Closed);
        let _ = cb.call(fail).await;
        assert_eq!(cb.state(), State::Open);

        // Subsequent calls short-circuit.
        let r = cb
            .call::<_, _, (), &'static str>(|| async { Ok(()) })
            .await;
        assert!(matches!(r, Err(BreakerError::Open)));
    }

    #[tokio::test]
    async fn half_open_closes_after_success() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            name: "test",
            failure_threshold: 1,
            cooldown: Duration::from_millis(20),
            success_threshold: 1,
        });

        let _ = cb.call(|| async { Err::<(), _>("x") }).await;
        assert_eq!(cb.state(), State::Open);

        tokio::time::sleep(Duration::from_millis(30)).await;

        // First post-cooldown call probes; success closes the breaker.
        let r = cb.call(|| async { Ok::<_, &str>(()) }).await;
        assert!(r.is_ok());
        assert_eq!(cb.state(), State::Closed);
    }
}
