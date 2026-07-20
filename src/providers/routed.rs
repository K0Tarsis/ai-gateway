use std::sync::Arc;

use async_trait::async_trait;

use crate::errors::GatewayError;
use crate::models::{ChatRequest, ChatResponse};
use crate::providers::{AiProvider, ChatStream};

// Wraps a vendor connection with a fixed model, so a route's name alone
// determines both provider and model — the caller never needs to know or
// send the underlying model string.
pub struct RoutedProvider {
    inner: Arc<dyn AiProvider>,
    model: String,
}

impl RoutedProvider {
    pub fn new(inner: Arc<dyn AiProvider>, model: String) -> Self {
        Self { inner, model }
    }
}

#[async_trait]
impl AiProvider for RoutedProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    async fn chat(&self, mut req: ChatRequest) -> Result<ChatResponse, GatewayError> {
        req.model = self.model.clone();
        self.inner.chat(req).await
    }

    async fn chat_stream(&self, mut req: ChatRequest) -> Result<ChatStream, GatewayError> {
        req.model = self.model.clone();
        self.inner.chat_stream(req).await
    }
}
