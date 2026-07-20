use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use metrics_exporter_prometheus::PrometheusHandle;
use sqlx::SqlitePool;

use crate::config::Config;
use crate::errors::GatewayError;
use crate::health::HealthTracker;
use crate::providers::AiProvider;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::azure::AzureProvider;
use crate::providers::openai::OpenAiProvider;
use crate::providers::routed::RoutedProvider;
use crate::ratelimit::RateLimiter;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub providers: Arc<HashMap<String, Arc<dyn AiProvider>>>,
    pub health: Arc<HealthTracker>,
    pub rate_limiter: Arc<RateLimiter>,
    // Cheap to clone internally (Arc-backed by the crate itself) — used only
    // to render the `/metrics` scrape response; recording happens via
    // `metrics::counter!`/`histogram!` macros against the global recorder
    // this handle was installed against, no state passing needed for that.
    pub metrics_handle: PrometheusHandle,
    // Also cheap to clone internally (a connection pool handle).
    pub db: SqlitePool,
}

impl AppState {
    // Fallible and async, unlike the rest of this constructor's in-memory
    // setup — SqlitePool::connect is a real I/O operation that can fail
    // (bad path, permissions), so it's propagated like config::load's
    // Result rather than treated as infallible.
    pub async fn new(config: Config) -> Result<Self, GatewayError> {
        let mut providers: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();

        if let Some(openai_config) = &config.providers.openai {
            if openai_config.enabled {
                let provider = OpenAiProvider::new(openai_config, &config.retry);
                providers.insert("openai".to_string(), Arc::new(provider));
            }
        }

        if let Some(anthropic_config) = &config.providers.anthropic {
            if anthropic_config.enabled {
                let provider = AnthropicProvider::new(anthropic_config, &config.retry);
                providers.insert("anthropic".to_string(), Arc::new(provider));
            }
        }

        if let Some(azure_config) = &config.providers.azure {
            if azure_config.enabled {
                let provider = AzureProvider::new(azure_config, &config.retry);
                providers.insert("azure".to_string(), Arc::new(provider));
            }
        }

        // Routes share the `providers` map's namespace: each one wraps an
        // already-built connection with a fixed model, so profile fallback
        // lists and the request-level `provider` override can name a route
        // exactly like they'd name a bare provider.
        for (route_name, route_config) in &config.routes {
            if let Some(base) = providers.get(&route_config.provider).cloned() {
                let routed = RoutedProvider::new(base, route_config.model.clone());
                providers.insert(route_name.clone(), Arc::new(routed));
            }
        }

        let health = HealthTracker::new(
            config.health.failure_threshold,
            Duration::from_secs(config.health.cooldown_secs),
        );

        let db = crate::db::connect(&config.database.path).await?;

        Ok(Self {
            config: Arc::new(config),
            providers: Arc::new(providers),
            health: Arc::new(health),
            rate_limiter: Arc::new(RateLimiter::new()),
            metrics_handle: crate::metrics::install(),
            db,
        })
    }
}
