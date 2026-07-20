use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

// Reactive circuit breaker: records success/failure after each real
// chat()/chat_stream() attempt the fallback loop already makes (no separate
// polling scheduler). After `failure_threshold` consecutive failures a
// provider is deprioritized in routing for `cooldown` — not removed outright,
// so a total outage still eventually gets tried rather than hard-erroring.
pub struct HealthTracker {
    failure_threshold: u32,
    cooldown: Duration,
    state: RwLock<HashMap<String, ProviderHealth>>,
}

#[derive(Default, Clone, Copy)]
struct ProviderHealth {
    consecutive_failures: u32,
    unhealthy_until: Option<Instant>,
}

impl HealthTracker {
    pub fn new(failure_threshold: u32, cooldown: Duration) -> Self {
        Self {
            failure_threshold,
            cooldown,
            state: RwLock::new(HashMap::new()),
        }
    }

    pub fn record_success(&self, name: &str) {
        let mut state = self.state.write().unwrap();
        state.insert(name.to_string(), ProviderHealth::default());
    }

    pub fn record_failure(&self, name: &str) {
        let mut state = self.state.write().unwrap();
        let entry = state.entry(name.to_string()).or_default();
        entry.consecutive_failures += 1;
        if entry.consecutive_failures >= self.failure_threshold {
            entry.unhealthy_until = Some(Instant::now() + self.cooldown);
        }
    }

    pub fn is_healthy(&self, name: &str) -> bool {
        let state = self.state.read().unwrap();
        match state.get(name).and_then(|h| h.unhealthy_until) {
            Some(until) => Instant::now() >= until,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_by_default() {
        let tracker = HealthTracker::new(3, Duration::from_secs(30));
        assert!(tracker.is_healthy("openai"));
    }

    #[test]
    fn trips_after_consecutive_failures() {
        let tracker = HealthTracker::new(3, Duration::from_secs(30));
        tracker.record_failure("openai");
        tracker.record_failure("openai");
        assert!(tracker.is_healthy("openai"));
        tracker.record_failure("openai");
        assert!(!tracker.is_healthy("openai"));
    }

    #[test]
    fn recovers_after_cooldown_elapses() {
        let tracker = HealthTracker::new(1, Duration::from_millis(20));
        tracker.record_failure("openai");
        assert!(!tracker.is_healthy("openai"));
        std::thread::sleep(Duration::from_millis(40));
        assert!(tracker.is_healthy("openai"));
    }

    #[test]
    fn success_resets_failure_count() {
        let tracker = HealthTracker::new(3, Duration::from_secs(30));
        tracker.record_failure("openai");
        tracker.record_failure("openai");
        tracker.record_success("openai");
        tracker.record_failure("openai");
        assert!(tracker.is_healthy("openai"));
    }

    #[test]
    fn tracks_providers_independently() {
        let tracker = HealthTracker::new(1, Duration::from_secs(30));
        tracker.record_failure("openai");
        assert!(!tracker.is_healthy("openai"));
        assert!(tracker.is_healthy("anthropic"));
    }
}
