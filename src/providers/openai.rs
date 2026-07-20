use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::config::{OpenAiConfig, RetryConfig};
use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse, ChatStreamChunk};
use crate::providers::retry::send_with_retry;
use crate::providers::sse::sse_data_lines;
use crate::providers::{AiProvider, ChatStream};

pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    base_url: String,
    max_retries: u32,
}

impl OpenAiProvider {
    pub fn new(config: &OpenAiConfig, retry: &RetryConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            max_retries: retry.max_attempts,
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

        let response = send_with_retry(
            || self.client.post(&url).bearer_auth(&self.api_key).json(&req),
            self.max_retries,
        )
        .await?;

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

    async fn chat_stream(&self, mut req: ChatRequest) -> Result<ChatStream, GatewayError> {
        req.stream = Some(true);
        let url = format!("{}/chat/completions", self.base_url);

        let response = send_with_retry(
            || self.client.post(&url).bearer_auth(&self.api_key).json(&req),
            self.max_retries,
        )
        .await?;

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

        // OpenAI's own SSE chunk shape already matches `ChatStreamChunk` 1:1.
        let stream = sse_data_lines(response.bytes_stream()).map(|payload| {
            let payload = payload?;
            serde_json::from_str::<ChatStreamChunk>(&payload).map_err(|e| {
                GatewayError::Provider(format!("openai stream chunk parse failed: {}", e))
            })
        });

        Ok(Box::pin(stream))
    }
}
