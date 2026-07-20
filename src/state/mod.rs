use std::collections::HashMap;
use std::sync::Arc;

use crate::config::Config;
use crate::providers::openai::OpenAiProvider;
use crate::providers::AiProvider;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub providers: Arc<HashMap<String, Arc<dyn AiProvider>>>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let mut providers: HashMap<String, Arc<dyn AiProvider>> = HashMap::new();

        if let Some(openai_config) = &config.providers.openai {
            if openai_config.enabled {
                let provider = OpenAiProvider::new(openai_config);
                providers.insert("openai".to_string(), Arc::new(provider));
            }
        }

        Self {
            config: Arc::new(config),
            providers: Arc::new(providers),
        }
    }
}
