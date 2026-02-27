//! Deterministic mock backend for tests and CI.
//!
//! Returns predictable responses based on the prompt content.
//! No external dependencies required.

use node_ai::{BackendHealth, GenerateRequest, GenerateResponse, InferenceBackend};

pub struct MockBackend {
    model_name: String,
}

impl MockBackend {
    pub fn new() -> Self {
        Self {
            model_name: "mock-v1".into(),
        }
    }

    pub fn with_model(model_name: &str) -> Self {
        Self {
            model_name: model_name.into(),
        }
    }
}

impl Default for MockBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl InferenceBackend for MockBackend {
    fn name(&self) -> &str {
        "mock"
    }

    async fn health_check(&self) -> anyhow::Result<BackendHealth> {
        Ok(BackendHealth {
            healthy: true,
            message: "mock backend always healthy".into(),
            model: self.model_name.clone(),
        })
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let prompt_lower = request.prompt.to_lowercase();

        let text = if prompt_lower.contains("hello") {
            "Hello! I'm MeshMind's mock AI assistant. How can I help you today?".into()
        } else if prompt_lower.contains("error") || prompt_lower.contains("fix") {
            "Based on the context provided, the issue is likely related to configuration. \
             Check logs and verify settings."
                .into()
        } else if prompt_lower.contains("summarize") || prompt_lower.contains("summary") {
            "Here is a brief summary of the provided context.".into()
        } else {
            format!(
                "Mock response to: {}",
                request.prompt.chars().take(100).collect::<String>()
            )
        };

        let tokens = (text.len() / 4) as u32;

        Ok(GenerateResponse {
            text,
            tokens_used: tokens.min(request.max_tokens),
            model: self.model_name.clone(),
            finish_reason: "stop".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn health_check() {
        let backend = MockBackend::new();
        let health = backend.health_check().await.unwrap();
        assert!(health.healthy);
        assert_eq!(health.model, "mock-v1");
    }

    #[tokio::test]
    async fn generate_hello() {
        let backend = MockBackend::new();
        let resp = backend
            .generate(GenerateRequest {
                prompt: "hello world".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.text.contains("Hello"));
        assert_eq!(resp.model, "mock-v1");
        assert_eq!(resp.finish_reason, "stop");
    }

    #[tokio::test]
    async fn generate_error_context() {
        let backend = MockBackend::new();
        let resp = backend
            .generate(GenerateRequest {
                prompt: "How do I fix this error in my service?".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.text.contains("configuration"));
    }

    #[tokio::test]
    async fn generate_generic() {
        let backend = MockBackend::new();
        let resp = backend
            .generate(GenerateRequest {
                prompt: "What is the capital of France?".into(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(resp.text.starts_with("Mock response to:"));
    }

    #[tokio::test]
    async fn custom_model_name() {
        let backend = MockBackend::with_model("test-model");
        let health = backend.health_check().await.unwrap();
        assert_eq!(health.model, "test-model");
    }

    #[test]
    fn backend_name() {
        let backend = MockBackend::new();
        assert_eq!(backend.name(), "mock");
    }
}
