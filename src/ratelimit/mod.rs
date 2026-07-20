use std::collections::HashMap;
use std::sync::RwLock;
use std::time::Instant;

// Per-profile token bucket, opt-in via `ProfileConfig.rate_limit`. Bucket
// capacity equals `requests_per_minute`, refilling continuously at that same
// rate — no separate burst allowance, since nothing has asked for one yet.
pub struct RateLimiter {
    state: RwLock<HashMap<String, Bucket>>,
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(HashMap::new()),
        }
    }

    // Returns true and consumes a token if the profile is under its limit,
    // false if it's currently exhausted.
    pub fn check(&self, profile_name: &str, requests_per_minute: u32) -> bool {
        let capacity = requests_per_minute as f64;
        let refill_per_sec = capacity / 60.0;
        let now = Instant::now();

        let mut state = self.state.write().unwrap();
        let bucket = state
            .entry(profile_name.to_string())
            .or_insert_with(|| Bucket {
                tokens: capacity,
                last_refill: now,
            });

        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(capacity);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn allows_up_to_capacity() {
        let limiter = RateLimiter::new();
        for _ in 0..5 {
            assert!(limiter.check("p", 5));
        }
        assert!(!limiter.check("p", 5));
    }

    #[test]
    fn refills_over_time() {
        let limiter = RateLimiter::new();
        let rpm = 120; // 2 tokens/sec
        for _ in 0..120 {
            assert!(limiter.check("p", rpm));
        }
        assert!(!limiter.check("p", rpm));
        sleep(Duration::from_millis(600)); // ~1 token refilled
        assert!(limiter.check("p", rpm));
    }

    #[test]
    fn tracks_profiles_independently() {
        let limiter = RateLimiter::new();
        for _ in 0..3 {
            assert!(limiter.check("a", 3));
        }
        assert!(!limiter.check("a", 3));
        assert!(limiter.check("b", 3));
    }
}
