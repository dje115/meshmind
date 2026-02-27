//! Ollama HTTP backend for InferenceBackend.
//!
//! Calls the Ollama REST API at a configurable endpoint.
//! Default: http://localhost:11434

use std::time::Duration;

use node_ai::{BackendHealth, GenerateRequest, GenerateResponse, InferenceBackend};
use reqwest::Client;
use serde::{Deserialize, Serialize};

pub struct OllamaBackend {
    client: Client,
    endpoint: String,
    model: String,
}

impl OllamaBackend {
    pub fn new(endpoint: &str, model: &str, timeout_ms: u64) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(timeout_ms))
            .build()?;
        Ok(Self {
            client,
            endpoint: endpoint.trim_end_matches('/').to_string(),
            model: model.to_string(),
        })
    }

    pub fn default_endpoint() -> Self {
        Self::new("http://localhost:11434", "llama3.2", 30_000)
            .expect("failed to create default OllamaBackend")
    }
}

#[derive(Serialize)]
struct OllamaGenerateRequest {
    model: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "is_zero_u32")]
    num_predict: u32,
    temperature: f32,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

#[derive(Deserialize)]
struct OllamaGenerateResponse {
    #[serde(default)]
    response: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    eval_count: u32,
}

#[derive(Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaModel>,
}

#[derive(Deserialize)]
struct OllamaModel {
    #[serde(default)]
    name: String,
}

#[async_trait::async_trait]
impl InferenceBackend for OllamaBackend {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn health_check(&self) -> anyhow::Result<BackendHealth> {
        let url = format!("{}/api/tags", self.endpoint);
        match self.client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let tags: OllamaTagsResponse = resp.json().await.unwrap_or(OllamaTagsResponse {
                    models: vec![],
                });
                let has_model = tags.models.iter().any(|m| m.name.starts_with(&self.model));
                Ok(BackendHealth {
                    healthy: true,
                    message: if has_model {
                        format!("model '{}' available", self.model)
                    } else {
                        format!(
                            "ollama running but model '{}' not found (available: {})",
                            self.model,
                            tags.models
                                .iter()
                                .map(|m| m.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    },
                    model: self.model.clone(),
                })
            }
            Ok(resp) => Ok(BackendHealth {
                healthy: false,
                message: format!("ollama returned status {}", resp.status()),
                model: self.model.clone(),
            }),
            Err(e) => Ok(BackendHealth {
                healthy: false,
                message: format!("cannot reach ollama at {}: {e}", self.endpoint),
                model: self.model.clone(),
            }),
        }
    }

    async fn generate(&self, request: GenerateRequest) -> anyhow::Result<GenerateResponse> {
        let url = format!("{}/api/generate", self.endpoint);

        let body = OllamaGenerateRequest {
            model: self.model.clone(),
            prompt: request.prompt,
            system: request.system,
            stream: false,
            options: OllamaOptions {
                num_predict: request.max_tokens,
                temperature: request.temperature,
                stop: request.stop,
            },
        };

        let resp = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("ollama returned {status}: {body_text}");
        }

        let ollama_resp: OllamaGenerateResponse = resp.json().await?;

        Ok(GenerateResponse {
            text: ollama_resp.response,
            tokens_used: ollama_resp.eval_count,
            model: ollama_resp.model,
            finish_reason: if ollama_resp.done { "stop" } else { "length" }.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_name() {
        let backend = OllamaBackend::default_endpoint();
        assert_eq!(backend.name(), "ollama");
    }

    #[test]
    fn custom_endpoint() {
        let backend = OllamaBackend::new("http://myhost:11434", "mistral", 60_000).unwrap();
        assert_eq!(backend.endpoint, "http://myhost:11434");
        assert_eq!(backend.model, "mistral");
    }

    #[tokio::test]
    async fn health_check_no_server() {
        let backend = OllamaBackend::new("http://127.0.0.1:19999", "test", 1000).unwrap();
        let health = backend.health_check().await.unwrap();
        assert!(!health.healthy);
        assert!(health.message.contains("cannot reach"));
    }
}
