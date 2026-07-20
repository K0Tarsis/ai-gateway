use async_trait::async_trait;

use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse};

pub mod openai;

#[async_trait]
pub trait AiProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, req: ChatRequest) -> Result<ChatResponse, GatewayError>;
}
