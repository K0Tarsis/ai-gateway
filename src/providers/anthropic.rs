use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::config::{AnthropicConfig, RetryConfig};
use crate::errors::GatewayError;
use crate::models::{
    ChatChoice, ChatMessage, ChatRequest, ChatResponse, ChatStreamChoice, ChatStreamChunk,
    ChatStreamDelta, Usage,
};
use crate::providers::retry::send_with_retry;
use crate::providers::sse::sse_data_lines;
use crate::providers::{AiProvider, ChatStream};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: u32 = 1024;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    base_url: String,
    max_retries: u32,
}

impl AnthropicProvider {
    pub fn new(config: &AnthropicConfig, retry: &RetryConfig) -> Self {
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

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct AnthropicChatRequest {
    model: String,
    max_tokens: u32,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    stream: bool,
}

#[derive(Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Deserialize)]
struct AnthropicChatResponse {
    id: String,
    model: String,
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

// Anthropic's stream events are internally tagged by `type`; only the three
// we act on are named, everything else (`content_block_start`,
// `content_block_stop`, `message_stop`, `ping`, ...) falls through to `Other`
// and is skipped.
#[derive(Deserialize)]
#[serde(tag = "type")]
enum AnthropicStreamEvent {
    #[serde(rename = "message_start")]
    MessageStart { message: AnthropicStreamMessage },
    #[serde(rename = "content_block_delta")]
    ContentBlockDelta { delta: AnthropicContentDelta },
    #[serde(rename = "message_delta")]
    MessageDelta { delta: AnthropicMessageDelta },
    #[serde(other)]
    Other,
}

#[derive(Deserialize)]
struct AnthropicStreamMessage {
    id: String,
    model: String,
}

#[derive(Deserialize)]
struct AnthropicContentDelta {
    #[serde(rename = "type")]
    delta_type: String,
    #[serde(default)]
    text: String,
}

#[derive(Deserialize)]
struct AnthropicMessageDelta {
    stop_reason: Option<String>,
}

// Anthropic has no top-level "system" role in `messages` — system messages
// are pulled out into a separate top-level field.
fn split_system_messages(messages: Vec<ChatMessage>) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system_parts = Vec::new();
    let mut rest = Vec::new();

    for message in messages {
        if message.role == "system" {
            system_parts.push(message.content);
        } else {
            rest.push(AnthropicMessage {
                role: message.role,
                content: message.content,
            });
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, rest)
}

fn map_stop_reason(reason: Option<&str>) -> Option<String> {
    reason.map(|r| {
        match r {
            "end_turn" | "stop_sequence" => "stop",
            "max_tokens" => "length",
            "tool_use" => "tool_calls",
            other => other,
        }
        .to_string()
    })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn empty_delta() -> ChatStreamDelta {
    ChatStreamDelta::default()
}

#[async_trait]
impl AiProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError> {
        let model = req.model.clone();
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let temperature = req.temperature;
        let (system, messages) = split_system_messages(req.messages);

        let body = AnthropicChatRequest {
            model,
            max_tokens,
            messages,
            system,
            temperature,
            stream: false,
        };

        let url = format!("{}/messages", self.base_url);
        let response = send_with_retry(
            || {
                self.client
                    .post(&url)
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .json(&body)
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
                "anthropic returned {}: {}",
                status, body
            )));
        }

        let parsed = response
            .json::<AnthropicChatResponse>()
            .await
            .map_err(|e| {
                GatewayError::Provider(format!("anthropic response parse failed: {}", e))
            })?;

        let text = parsed
            .content
            .iter()
            .filter(|block| block.block_type == "text")
            .map(|block| block.text.as_str())
            .collect::<Vec<_>>()
            .join("");

        Ok(ChatResponse {
            id: parsed.id,
            object: "chat.completion".to_string(),
            created: unix_now(),
            model: parsed.model,
            choices: vec![ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant".to_string(),
                    content: text,
                },
                finish_reason: map_stop_reason(parsed.stop_reason.as_deref()),
            }],
            usage: Some(Usage {
                prompt_tokens: parsed.usage.input_tokens,
                completion_tokens: parsed.usage.output_tokens,
                total_tokens: parsed.usage.input_tokens + parsed.usage.output_tokens,
            }),
        })
    }

    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, GatewayError> {
        let requested_model = req.model.clone();
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let temperature = req.temperature;
        let (system, messages) = split_system_messages(req.messages);

        let body = AnthropicChatRequest {
            model: requested_model.clone(),
            max_tokens,
            messages,
            system,
            temperature,
            stream: true,
        };

        let url = format!("{}/messages", self.base_url);
        let response = send_with_retry(
            || {
                self.client
                    .post(&url)
                    .header("x-api-key", &self.api_key)
                    .header("anthropic-version", ANTHROPIC_VERSION)
                    .json(&body)
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
                "anthropic returned {}: {}",
                status, body
            )));
        }

        let created = unix_now();
        let events = sse_data_lines(response.bytes_stream());

        let stream = async_stream::try_stream! {
            futures::pin_mut!(events);

            let mut id = String::new();
            let mut model = requested_model;

            while let Some(payload) = events.next().await {
                let payload = payload?;
                let event: AnthropicStreamEvent = match serde_json::from_str(&payload) {
                    Ok(event) => event,
                    Err(_) => continue,
                };

                let choice = match event {
                    AnthropicStreamEvent::MessageStart { message } => {
                        id = message.id;
                        model = message.model;
                        ChatStreamChoice {
                            index: 0,
                            delta: ChatStreamDelta {
                                role: Some("assistant".to_string()),
                                content: None,
                            },
                            finish_reason: None,
                        }
                    }
                    AnthropicStreamEvent::ContentBlockDelta { delta } if delta.delta_type == "text_delta" => {
                        ChatStreamChoice {
                            index: 0,
                            delta: ChatStreamDelta {
                                role: None,
                                content: Some(delta.text),
                            },
                            finish_reason: None,
                        }
                    }
                    AnthropicStreamEvent::MessageDelta { delta } => ChatStreamChoice {
                        index: 0,
                        delta: empty_delta(),
                        finish_reason: map_stop_reason(delta.stop_reason.as_deref()),
                    },
                    _ => continue,
                };

                yield ChatStreamChunk {
                    id: id.clone(),
                    object: "chat.completion.chunk".to_string(),
                    created,
                    model: model.clone(),
                    choices: vec![choice],
                };
            }
        };

        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_system_messages_out() {
        let messages = vec![
            ChatMessage {
                role: "system".to_string(),
                content: "be terse".to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
            },
        ];

        let (system, rest) = split_system_messages(messages);
        assert_eq!(system, Some("be terse".to_string()));
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].role, "user");
    }

    #[test]
    fn no_system_message_yields_none() {
        let messages = vec![ChatMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
        }];

        let (system, rest) = split_system_messages(messages);
        assert_eq!(system, None);
        assert_eq!(rest.len(), 1);
    }

    #[test]
    fn maps_known_stop_reasons() {
        assert_eq!(map_stop_reason(Some("end_turn")), Some("stop".to_string()));
        assert_eq!(
            map_stop_reason(Some("max_tokens")),
            Some("length".to_string())
        );
        assert_eq!(map_stop_reason(None), None);
    }
}
