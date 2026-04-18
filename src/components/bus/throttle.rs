use std::time::Instant;

use tokio::time::Duration;

pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    rate: f64, // tokens per second
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(rate_per_sec: u32, burst: u32) -> Self {
        TokenBucket {
            capacity: burst as f64,
            tokens: burst as f64,
            rate: rate_per_sec as f64,
            last_refill: Instant::now(),
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.rate).min(self.capacity);
        self.last_refill = now;
    }

    /// Try to consume one token. Returns `None` if a token was available (caller may proceed),
    /// or `Some(duration)` indicating how long to wait before trying again.
    /// Does not block — the caller is responsible for sleeping outside the lock.
    pub fn try_acquire(&mut self) -> Option<Duration> {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            None
        } else {
            Some(Duration::from_secs_f64((1.0 - self.tokens) / self.rate))
        }
    }
}

pub struct Throttle {
    pub rate: u32,  // msg/sec
    pub burst: u32,
}

impl Throttle {
    pub fn new(rate: u32, burst: u32) -> Self {
        Throttle { rate, burst }
    }
}
