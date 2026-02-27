//! InferenceBackend trait, factory, and prompt assembly helpers.
//!
//! The backend has no direct DB or mesh access.
//! Prompt assembly and retrieval happen in the application layer.

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct GenerateRequest {
    pub prompt: String,
    pub system: Option<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub stop: Vec<String>,
}

impl Default for GenerateRequest {
    fn default() -> Self {
        Self {
            prompt: String::new(),
            system: None,
            max_tokens: 1024,
            temperature: 0.7,
            stop: vec![],
        }
    }
}

#[derive(Debug, Clone)]
pub struct GenerateResponse {
    pub text: String,
    pub tokens_used: u32,
    pub model: String,
    pub finish_reason: String,
}

#[derive(Debug, Clone)]
pub struct BackendHealth {
    pub healthy: bool,
    pub message: String,
    pub model: String,
}

/// The pluggable inference backend trait.
#[async_trait::async_trait]
pub trait InferenceBackend: Send + Sync {
    /// Human-readable backend name (e.g. "ollama", "mock").
    fn name(&self) -> &str;

    /// Check if the backend is available and healthy.
    async fn health_check(&self) -> anyhow::Result<BackendHealth>;

    /// Generate a response from a prompt.
    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse>;
}

/// Configuration for selecting and configuring a backend.
#[derive(Debug, Clone)]
pub struct BackendConfig {
    pub backend: String,
    pub endpoint: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_tokens: u32,
    pub extra: HashMap<String, String>,
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend: "mock".into(),
            endpoint: "http://localhost:11434".into(),
            model: "llama3.2".into(),
            timeout_ms: 30_000,
            max_tokens: 1024,
            extra: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = BackendConfig::default();
        assert_eq!(config.backend, "mock");
        assert_eq!(config.max_tokens, 1024);
    }

    #[test]
    fn default_request() {
        let req = GenerateRequest::default();
        assert_eq!(req.max_tokens, 1024);
        assert!((req.temperature - 0.7).abs() < f32::EPSILON);
    }
}
