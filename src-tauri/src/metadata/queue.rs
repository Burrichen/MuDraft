//! One shared request queue per provider. Callers wait their turn (FIFO: tokio's mutex is
//! fair) and requests leave at most once per `interval`. A 503/429 pushes the next slot
//! back so every queued caller backs off together.

use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::error::{AppError, AppResult};

pub struct RateLimiter {
    next_slot: Mutex<Option<Instant>>,
    interval: Duration,
}

impl RateLimiter {
    pub fn new(interval: Duration) -> Self {
        Self {
            next_slot: Mutex::new(None),
            interval,
        }
    }

    /// Wait for this caller's slot. Cancelling while queued never sends the request.
    pub async fn acquire(&self, cancel: &CancellationToken) -> AppResult<()> {
        let mut next = tokio::select! {
            guard = self.next_slot.lock() => guard,
            () = cancel.cancelled() => return Err(AppError::Cancelled),
        };
        if let Some(at) = *next
            && at > Instant::now()
        {
            tokio::select! {
                () = sleep_until(at) => {}
                () = cancel.cancelled() => return Err(AppError::Cancelled),
            }
        }
        *next = Some(Instant::now() + self.interval);
        Ok(())
    }

    /// Delay everyone's next request by at least `delay` from now.
    pub async fn penalize(&self, delay: Duration) {
        let mut next = self.next_slot.lock().await;
        let until = Instant::now() + delay;
        if next.is_none_or(|at| at < until) {
            *next = Some(until);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn spaces_requests_by_the_interval() {
        let limiter = RateLimiter::new(Duration::from_secs(1));
        let cancel = CancellationToken::new();
        let start = Instant::now();
        for _ in 0..3 {
            limiter.acquire(&cancel).await.unwrap();
        }
        assert_eq!(start.elapsed(), Duration::from_secs(2));
    }

    #[tokio::test(start_paused = true)]
    async fn penalty_delays_the_next_slot_and_cancel_leaves_the_queue() {
        let limiter = RateLimiter::new(Duration::from_secs(1));
        let cancel = CancellationToken::new();
        limiter.acquire(&cancel).await.unwrap();
        limiter.penalize(Duration::from_secs(5)).await;
        let start = Instant::now();
        limiter.acquire(&cancel).await.unwrap();
        assert_eq!(start.elapsed(), Duration::from_secs(5));

        let waiting = CancellationToken::new();
        waiting.cancel();
        assert_eq!(
            limiter.acquire(&waiting).await.unwrap_err(),
            AppError::Cancelled
        );
    }
}
