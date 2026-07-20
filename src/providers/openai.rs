use async_trait::async_trait;
use reqwest::Client;

use crate::config::OpenAiConfig;
use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse};
use crate::providers::AiProvider;

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(config: &OpenAiConfig) -> Self {
        Self {
            client: Client::new(),
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
        }
    }
}

#[async_trait]
impl AiProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError> {
        let url = format!("{}/chat/completions", self.base_url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await
            .map_err(|e| GatewayError::Provider(format!("openai request failed: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<no response body>".to_string());
            return Err(GatewayError::Provider(format!(
                "openai returned {}: {}",
                status, body
            )));
        }

        response
            .json::<ChatResponse>()
            .await
            .map_err(|e| GatewayError::Provider(format!("openai response parse failed: {}", e)))
    }
}
