use std::sync::Arc;

use crate::errors::GatewayError;
use crate::providers::AiProvider;
use crate::state::AppState;

// Picks the first enabled provider in the configured fallback chain.
// Model-name-aware routing lands once more than one provider exists.
pub fn select_provider(state: &AppState) -> Result<Arc<dyn AiProvider>, GatewayError> {
    for name in &state.config.routing.fallback {
        if let Some(provider) = state.providers.get(name) {
            return Ok(provider.clone());
        }
    }

    Err(GatewayError::NotFound(
        "no enabled provider found in routing.fallback".to_string(),
    ))
}
