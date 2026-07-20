use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;

use crate::config::{AzureConfig, RetryConfig};
use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse, ChatStreamChunk};
use crate::providers::retry::send_with_retry;
use crate::providers::sse::sse_data_lines;
use crate::providers::{AiProvider, ChatStream};

pub struct AzureProvider {
    client: Client,
    api_key: String,
    base_url: String,
    api_version: String,
    max_retries: u32,
}

impl AzureProvider {
    pub fn new(config: &AzureConfig, retry: &RetryConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            api_version: config.api_version.clone(),
            max_retries: retry.max_attempts,
        }
    }
}

// Azure addresses a specific deployment, not a bare model string:
// {base_url}/openai/deployments/{deployment}/chat/completions?api-version=...
// The deployment name is whatever's in `req.model` — pinned via a `routes:`
// entry the same way AnthropicProvider treats req.model as its wire-format
// model string.
fn deployment_url(base_url: &str, deployment: &str, api_version: &str) -> String {
    format!(
        "{}/openai/deployments/{}/chat/completions?api-version={}",
        base_url.trim_end_matches('/'),
        deployment,
        api_version
    )
}

#[async_trait]
impl AiProvider for AzureProvider {
    fn name(&self) -> &str {
        "azure"
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError> {
        let url = deployment_url(&self.base_url, &req.model, &self.api_version);

        let response = send_with_retry(
            || {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .json(&req)
            },
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
                "azure returned {}: {}",
                status, body
            )));
        }

        response
            .json::<ChatResponse>()
            .await
            .map_err(|e| GatewayError::Provider(format!("azure response parse failed: {}", e)))
    }

    async fn chat_stream(&self, mut req: ChatRequest) -> Result<ChatStream, GatewayError> {
        req.stream = Some(true);
        let url = deployment_url(&self.base_url, &req.model, &self.api_version);

        let response = send_with_retry(
            || {
                self.client
                    .post(&url)
                    .header("api-key", &self.api_key)
                    .json(&req)
            },
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
                "azure returned {}: {}",
                status, body
            )));
        }

        // Azure's chat-completions SSE shape already matches `ChatStreamChunk`
        // 1:1, same as OpenAI's.
        let stream = sse_data_lines(response.bytes_stream()).map(|payload| {
            let payload = payload?;
            serde_json::from_str::<ChatStreamChunk>(&payload).map_err(|e| {
                GatewayError::Provider(format!("azure stream chunk parse failed: {}", e))
            })
        });

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_deployment_url_with_api_version() {
        let url = deployment_url(
            "https://my-resource.openai.azure.com",
            "my-gpt4o-deployment",
            "2024-06-01",
        );
        assert_eq!(
            url,
            "https://my-resource.openai.azure.com/openai/deployments/my-gpt4o-deployment/chat/completions?api-version=2024-06-01"
        );
    }

    #[test]
    fn strips_trailing_slash_from_base_url() {
        let url = deployment_url("https://my-resource.openai.azure.com/", "dep", "2024-06-01");
        assert_eq!(
            url,
            "https://my-resource.openai.azure.com/openai/deployments/dep/chat/completions?api-version=2024-06-01"
        );
    }
}
