use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse, ChatStreamChunk};

pub mod anthropic;
pub mod azure;
pub mod openai;
pub mod retry;
pub mod routed;
pub mod sse;

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatStreamChunk, GatewayError>> + Send>>;

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError>;
    async fn chat_stream(&self, req: ChatRequest) -> Result<ChatStream, GatewayError>;
}
