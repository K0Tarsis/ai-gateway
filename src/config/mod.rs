use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::errors::GatewayError;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub profiles: Vec<ProfileConfig>,
    #[serde(default)]
    pub providers: ProvidersConfig,
    // Named shortcuts that pin a connection to a fixed model (e.g.
    // "anthropic-opus" -> {provider: anthropic, model: claude-opus-4...}).
    // Routes live in the same namespace as provider names, so they can be
    // used anywhere a provider name is used: profile fallback lists and the
    // request-level `provider` override.
    #[serde(default)]
    pub routes: HashMap<String, RouteConfig>,
    #[serde(default)]
    pub retry: RetryConfig,
    #[serde(default)]
    pub health: HealthConfig,
    #[serde(default)]
    pub database: DatabaseConfig,
    // Keyed by the exact model string returned in a provider's response
    // (not necessarily what a client requested — see `estimate_cost`).
    // Absent entries mean "unpriced," not "free".
    #[serde(default)]
    pub pricing: HashMap<String, ModelPricing>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RouteConfig {
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProfileConfig {
    pub name: String,
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub allowed_ips: Vec<String>,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    #[serde(default)]
    pub cost_limit: Option<CostLimitConfig>,
}

// Opt-in per-profile request cap. Absent means unlimited — most profiles
// (e.g. a trusted desktop client) don't need one.
#[derive(Debug, Deserialize, Clone)]
pub struct RateLimitConfig {
    pub requests_per_minute: u32,
}

// Opt-in per-profile spend cap, checked against request_log before each
// chat completion. Either/both bounds may be set; absent means no cap on
// that cadence. Absent struct entirely means unlimited, same as rate_limit.
#[derive(Debug, Deserialize, Clone)]
pub struct CostLimitConfig {
    #[serde(default)]
    pub daily_usd: Option<f64>,
    #[serde(default)]
    pub monthly_usd: Option<f64>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProvidersConfig {
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub ollama: Option<OllamaConfig>,
    pub azure: Option<AzureConfig>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiConfig {
    pub enabled: bool,
    pub api_key: String,
    #[serde(default = "default_openai_base_url")]
    pub base_url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicConfig {
    pub enabled: bool,
    pub api_key: String,
    #[serde(default = "default_anthropic_base_url")]
    pub base_url: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct OllamaConfig {
    pub enabled: bool,
    pub base_url: String,
}

// `base_url` is the Azure resource endpoint (e.g.
// https://{resource}.openai.azure.com) — no sensible cross-tenant default,
// unlike openai/anthropic's base_url. The model actually reached is a
// deployment name, addressed via routes: (see AzureProvider) rather than a
// bare model string.
#[derive(Debug, Deserialize)]
pub struct AzureConfig {
    pub enabled: bool,
    pub api_key: String,
    pub base_url: String,
    #[serde(default = "default_azure_api_version")]
    pub api_version: String,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct RoutingConfig {
    #[serde(default)]
    pub fallback: Vec<String>,
}

// Same-provider retry (connection errors, 429, 5xx) before a candidate counts
// as failed for cross-provider failover purposes.
#[derive(Debug, Deserialize, Clone)]
pub struct RetryConfig {
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: default_max_attempts(),
        }
    }
}

// Circuit breaker thresholds: after `failure_threshold` consecutive failures a
// provider is deprioritized in routing for `cooldown_secs`.
#[derive(Debug, Deserialize, Clone)]
pub struct HealthConfig {
    #[serde(default = "default_failure_threshold")]
    pub failure_threshold: u32,
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            failure_threshold: default_failure_threshold(),
            cooldown_secs: default_cooldown_secs(),
        }
    }
}

// Path to the SQLite database file backing request-log persistence.
#[derive(Debug, Deserialize, Clone)]
pub struct DatabaseConfig {
    #[serde(default = "default_database_path")]
    pub path: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_database_path(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ModelPricing {
    pub prompt_price_per_million: f64,
    pub completion_price_per_million: f64,
}

impl Config {
    // Returns `None` when no pricing entry exists for `model` — "unpriced,"
    // distinct from a genuine $0 cost.
    pub fn estimate_cost(
        &self,
        model: &str,
        prompt_tokens: u32,
        completion_tokens: u32,
    ) -> Option<f64> {
        let pricing = self.pricing.get(model)?;
        Some(
            (prompt_tokens as f64 / 1_000_000.0) * pricing.prompt_price_per_million
                + (completion_tokens as f64 / 1_000_000.0) * pricing.completion_price_per_million,
        )
    }
}

pub fn load(path: &str) -> Result<Config, GatewayError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| GatewayError::Config(format!("Cannot read config file '{}': {}", path, e)))?;

    let substituted = substitute_env_vars(&raw)?;

    let config: Config = serde_yaml::from_str(&substituted)
        .map_err(|e| GatewayError::Config(format!("Invalid config: {}", e)))?;

    validate(&config)?;

    Ok(config)
}

// A fallback list spanning more than one underlying vendor must name only
// routes, never a bare provider. A bare provider forwards the client's
// `model` field upstream unchanged (see AiProvider impls), which is only
// safe when every candidate in the chain shares one vendor's model
// catalog — otherwise a failover can silently hand one vendor's model name
// to another vendor's API. A route sidesteps this by overwriting `model`
// with its own pinned value (see RoutedProvider::chat), so it's always
// safe to mix vendors as long as every entry is a route.
fn validate(config: &Config) -> Result<(), GatewayError> {
    fn vendor_of<'a>(config: &'a Config, entry: &'a str) -> &'a str {
        config
            .routes
            .get(entry)
            .map(|route| route.provider.as_str())
            .unwrap_or(entry)
    }

    for profile in &config.profiles {
        let vendors: HashSet<&str> = profile
            .routing
            .fallback
            .iter()
            .map(|entry| vendor_of(config, entry))
            .collect();

        if vendors.len() > 1
            && let Some(bare) = profile
                .routing
                .fallback
                .iter()
                .find(|entry| !config.routes.contains_key(entry.as_str()))
        {
            return Err(GatewayError::Config(format!(
                "profile '{}': routing.fallback spans multiple providers {:?} but '{}' is a bare \
                 provider, not a route -- a fallback chain mixing vendors must name only `routes:` \
                 entries, since a bare provider forwards the client's `model` field upstream \
                 unchanged and it may not be valid on every vendor in the chain",
                profile.name, vendors, bare
            )));
        }
    }

    Ok(())
}

// Replaces ${VAR_NAME} placeholders with values from the environment.
fn substitute_env_vars(content: &str) -> Result<String, GatewayError> {
    let mut result = content.to_string();
    loop {
        let start = match result.find("${") {
            Some(i) => i,
            None => break,
        };
        let end = match result[start..].find('}') {
            Some(i) => start + i,
            None => return Err(GatewayError::Config("Unclosed ${ in config file".into())),
        };
        let var_name = result[start + 2..end].to_string();
        let value = std::env::var(&var_name).map_err(|_| {
            GatewayError::Config(format!("Environment variable '{}' not set", var_name))
        })?;
        result = format!("{}{}{}", &result[..start], value, &result[end + 1..]);
    }
    Ok(result)
}

fn default_openai_base_url() -> String {
    "https://api.openai.com/v1".to_string()
}

fn default_anthropic_base_url() -> String {
    "https://api.anthropic.com/v1".to_string()
}

fn default_timeout_secs() -> u64 {
    30
}

fn default_azure_api_version() -> String {
    "2024-06-01".to_string()
}

fn default_max_attempts() -> u32 {
    2
}

fn default_failure_threshold() -> u32 {
    3
}

fn default_cooldown_secs() -> u64 {
    30
}

fn default_database_path() -> String {
    "gateway.db".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(profiles: Vec<ProfileConfig>, routes: HashMap<String, RouteConfig>) -> Config {
        Config {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
            },
            profiles,
            providers: ProvidersConfig::default(),
            routes,
            retry: RetryConfig::default(),
            health: HealthConfig::default(),
            database: DatabaseConfig::default(),
            pricing: HashMap::new(),
        }
    }

    fn profile(name: &str, fallback: &[&str]) -> ProfileConfig {
        ProfileConfig {
            name: name.to_string(),
            api_keys: vec![],
            allowed_ips: vec![],
            routing: RoutingConfig {
                fallback: fallback.iter().map(|s| s.to_string()).collect(),
            },
            rate_limit: None,
            cost_limit: None,
        }
    }

    fn route(provider: &str, model: &str) -> RouteConfig {
        RouteConfig {
            provider: provider.to_string(),
            model: model.to_string(),
        }
    }

    #[test]
    fn single_vendor_bare_fallback_is_valid() {
        let config = test_config(vec![profile("desktop", &["openai"])], HashMap::new());
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn single_vendor_mixing_bare_entry_and_route_is_valid() {
        let routes = HashMap::from([("openai-mini".to_string(), route("openai", "gpt-4o-mini"))]);
        let config = test_config(vec![profile("desktop", &["openai", "openai-mini"])], routes);
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn multi_vendor_fallback_of_only_routes_is_valid() {
        let routes = HashMap::from([
            ("openai-mini".to_string(), route("openai", "gpt-4o-mini")),
            (
                "anthropic-sonnet".to_string(),
                route("anthropic", "claude-sonnet-4-20250514"),
            ),
        ]);
        let config = test_config(
            vec![profile("partner-a", &["anthropic-sonnet", "openai-mini"])],
            routes,
        );
        assert!(validate(&config).is_ok());
    }

    #[test]
    fn multi_vendor_fallback_with_a_bare_provider_is_rejected() {
        let routes = HashMap::from([(
            "anthropic-sonnet".to_string(),
            route("anthropic", "claude-sonnet-4-20250514"),
        )]);
        let config = test_config(
            vec![profile("partner-a", &["anthropic-sonnet", "openai"])],
            routes,
        );
        let err = validate(&config).unwrap_err();
        assert!(err.to_string().contains("openai"));
    }

    #[test]
    fn multi_vendor_fallback_of_two_bare_providers_is_rejected() {
        let config = test_config(
            vec![profile("partner-a", &["anthropic", "openai"])],
            HashMap::new(),
        );
        assert!(validate(&config).is_err());
    }
}
