//! The only place the program learns the time. Everything else is handed a `Clock`.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug)]
pub struct Clock(Arc<Source>);

#[derive(Debug)]
enum Source {
    /// Wall time read once, then moved on by tokio's `Instant`, so a paused tokio clock in
    /// tests moves this clock too and the simulator's timers agree with it.
    Tokio {
        base_ms: i64,
        start: tokio::time::Instant,
    },
    Manual(AtomicI64),
}

impl Clock {
    /// Must be called inside a tokio runtime.
    pub fn system() -> Self {
        let base_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Clock(Arc::new(Source::Tokio {
            base_ms,
            start: tokio::time::Instant::now(),
        }))
    }

    /// A clock that stands still until `advance` is called.
    pub fn manual(start_ms: i64) -> Self {
        Clock(Arc::new(Source::Manual(AtomicI64::new(start_ms))))
    }

    pub fn now_ms(&self) -> i64 {
        match &*self.0 {
            Source::Tokio { base_ms, start } => base_ms + start.elapsed().as_millis() as i64,
            Source::Manual(ms) => ms.load(Ordering::SeqCst),
        }
    }

    /// Moves a manual clock. A tokio-backed clock is moved with `tokio::time::advance`
    /// instead, so this refuses rather than letting the two disagree.
    pub fn advance(&self, ms: i64) -> Result<(), &'static str> {
        match &*self.0 {
            Source::Manual(now) => {
                now.fetch_add(ms, Ordering::SeqCst);
                Ok(())
            }
            Source::Tokio { .. } => Err("a tokio-backed clock moves with tokio::time::advance"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_clock_moves_only_by_hand() {
        let c = Clock::manual(1_000);
        assert_eq!(c.now_ms(), 1_000);
        c.advance(500).unwrap();
        assert_eq!(c.now_ms(), 1_500);
    }

    #[tokio::test(start_paused = true)]
    async fn system_clock_follows_paused_tokio_time() {
        let c = Clock::system();
        let before = c.now_ms();
        tokio::time::advance(std::time::Duration::from_secs(6)).await;
        assert_eq!(c.now_ms() - before, 6_000);
        assert!(c.advance(1).is_err());
    }
}
