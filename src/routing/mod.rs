use std::collections::HashMap;
use std::sync::Arc;

use crate::errors::GatewayError;
use crate::health::HealthTracker;
use crate::providers::AiProvider;

// Builds the ordered list of providers to try for a request: the client's
// requested provider first (if any and if permitted by the profile), then
// the rest of the profile's fallback chain, with providers currently tripped
// by the health tracker deprioritized to the end (not dropped — a total
// outage should still eventually be attempted). Runtime errors from each
// candidate fall through to the next one — see handlers::chat_completions.
pub fn select_providers(
    providers: &HashMap<String, Arc<dyn AiProvider>>,
    health: &HealthTracker,
    fallback: &[String],
    requested: Option<&str>,
) -> Result<Vec<Arc<dyn AiProvider>>, GatewayError> {
    let order = build_order(fallback, requested)?;

    let resolved: Vec<Arc<dyn AiProvider>> = order
        .into_iter()
        .filter_map(|name| providers.get(&name).cloned())
        .collect();

    if resolved.is_empty() {
        return Err(GatewayError::NotFound(
            "no enabled provider found in profile's routing.fallback".to_string(),
        ));
    }

    // Health is tracked per underlying vendor connection (`provider.name()`,
    // e.g. "anthropic"), not per route alias — a vendor outage should
    // deprioritize every route backed by it, not just one model alias.
    let (healthy, unhealthy): (Vec<_>, Vec<_>) = resolved
        .into_iter()
        .partition(|p| health.is_healthy(p.name()));

    Ok(healthy.into_iter().chain(unhealthy).collect())
}

// A requested provider must already be present in the profile's own
// fallback list — that list is the tenant's allowlist, so pinning to
// something outside it would let a client reach providers/routes their
// profile was never granted.
fn build_order(fallback: &[String], requested: Option<&str>) -> Result<Vec<String>, GatewayError> {
    match requested {
        Some(name) => {
            if !fallback.iter().any(|f| f == name) {
                return Err(GatewayError::NotFound(format!(
                    "provider '{}' is not permitted for this profile",
                    name
                )));
            }
            let mut order = vec![name.to_string()];
            order.extend(fallback.iter().filter(|f| f.as_str() != name).cloned());
            Ok(order)
        }
        None => Ok(fallback.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChatRequest, ChatResponse};
    use crate::providers::ChatStream;
    use async_trait::async_trait;
    use std::time::Duration;

    struct StubProvider {
        name: &'static str,
    }

    #[async_trait]
    impl AiProvider for StubProvider {
        fn name(&self) -> &str {
            self.name
        }

        async fn chat(&self, _req: ChatRequest) -> Result<ChatResponse, GatewayError> {
            unimplemented!()
        }

        async fn chat_stream(&self, _req: ChatRequest) -> Result<ChatStream, GatewayError> {
            unimplemented!()
        }
    }

    fn providers(names: &[&'static str]) -> HashMap<String, Arc<dyn AiProvider>> {
        names
            .iter()
            .map(|name| {
                (
                    name.to_string(),
                    Arc::new(StubProvider { name }) as Arc<dyn AiProvider>,
                )
            })
            .collect()
    }

    fn tracker() -> HealthTracker {
        HealthTracker::new(3, Duration::from_secs(30))
    }

    #[test]
    fn picks_first_available_provider_when_none_requested() {
        let providers = providers(&["openai", "anthropic"]);
        let fallback = vec!["anthropic".to_string(), "openai".to_string()];
        let selected = select_providers(&providers, &tracker(), &fallback, None).unwrap();
        assert_eq!(selected[0].name(), "anthropic");
        assert_eq!(selected[1].name(), "openai");
    }

    #[test]
    fn skips_fallback_entries_not_in_map() {
        let providers = providers(&["openai"]);
        let fallback = vec!["anthropic".to_string(), "openai".to_string()];
        let selected = select_providers(&providers, &tracker(), &fallback, None).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name(), "openai");
    }

    #[test]
    fn errors_when_no_fallback_entry_is_available() {
        let providers = providers(&["openai"]);
        let fallback = vec!["anthropic".to_string()];
        assert!(select_providers(&providers, &tracker(), &fallback, None).is_err());
    }

    #[test]
    fn requested_provider_is_tried_first() {
        let providers = providers(&["openai", "anthropic"]);
        let fallback = vec!["openai".to_string(), "anthropic".to_string()];
        let selected =
            select_providers(&providers, &tracker(), &fallback, Some("anthropic")).unwrap();
        assert_eq!(selected[0].name(), "anthropic");
        assert_eq!(selected[1].name(), "openai");
    }

    #[test]
    fn requested_provider_outside_fallback_is_rejected() {
        let providers = providers(&["openai", "anthropic"]);
        let fallback = vec!["openai".to_string()];
        let result = select_providers(&providers, &tracker(), &fallback, Some("anthropic"));
        assert!(result.is_err());
    }

    #[test]
    fn unhealthy_provider_is_deprioritized_not_dropped() {
        let providers = providers(&["openai", "anthropic"]);
        let fallback = vec!["openai".to_string(), "anthropic".to_string()];
        let health = tracker();
        health.record_failure("openai");
        health.record_failure("openai");
        health.record_failure("openai");

        let selected = select_providers(&providers, &health, &fallback, None).unwrap();
        assert_eq!(selected[0].name(), "anthropic");
        assert_eq!(selected[1].name(), "openai");
    }

    #[test]
    fn all_unhealthy_still_returns_full_list() {
        let providers = providers(&["openai", "anthropic"]);
        let fallback = vec!["openai".to_string(), "anthropic".to_string()];
        let health = tracker();
        for name in ["openai", "anthropic"] {
            health.record_failure(name);
            health.record_failure(name);
            health.record_failure(name);
        }

        let selected = select_providers(&providers, &health, &fallback, None).unwrap();
        assert_eq!(selected.len(), 2);
    }
}
