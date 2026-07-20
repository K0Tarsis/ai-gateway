use serde::Deserialize;

use crate::errors::GatewayError;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub security: SecurityConfig,
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub routing: RoutingConfig,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct SecurityConfig {
    pub api_keys: Vec<String>,
    #[serde(default)]
    pub allowed_ips: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProvidersConfig {
    pub openai: Option<OpenAiConfig>,
    pub anthropic: Option<AnthropicConfig>,
    pub ollama: Option<OllamaConfig>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAiConfig {
    pub enabled: bool,
    pub api_key: String,
    #[serde(default = "default_openai_base_url")]
    pub base_url: String,
}

#[derive(Debug, Deserialize)]
pub struct AnthropicConfig {
    pub enabled: bool,
    pub api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OllamaConfig {
    pub enabled: bool,
    pub base_url: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct RoutingConfig {
    #[serde(default)]
    pub fallback: Vec<String>,
}

pub fn load(path: &str) -> Result<Config, GatewayError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        GatewayError::Config(format!("Cannot read config file '{}': {}", path, e))
    })?;

    let substituted = substitute_env_vars(&raw)?;

    serde_yaml::from_str(&substituted)
        .map_err(|e| GatewayError::Config(format!("Invalid config: {}", e)))
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
