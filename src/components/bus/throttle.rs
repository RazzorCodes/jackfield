use std::time::Instant;

use tokio::time::{sleep, Duration};

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

    pub async fn acquire(&mut self) {
        loop {
            self.refill();
            if self.tokens >= 1.0 {
                self.tokens -= 1.0;
                return;
            }
            // Calculate how long to wait for 1 token
            let wait_secs = (1.0 - self.tokens) / self.rate;
            let wait_duration = Duration::from_secs_f64(wait_secs);
            sleep(wait_duration).await;
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
