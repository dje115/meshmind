//! Web research with citation extraction, WebBrief artifacts, and policy gating.
//!
//! Flow:
//! 1. Policy check: can_research_web()
//! 2. Fetch URL
//! 3. Extract/summarize with inference backend
//! 4. Store WebBrief as event
//! 5. Optionally redact before sharing

use std::sync::Arc;

use node_ai::InferenceBackend;
use node_policy::{PolicyDecision, PolicyEngine};
use node_proto::common::*;
use node_proto::events::*;
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchRequest {
    pub url: String,
    pub question: String,
    pub tenant_id: String,
    pub allow_web: bool,
    pub redaction_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchResult {
    pub artifact_id: String,
    pub question: String,
    pub summary: String,
    pub sources: Vec<String>,
    pub confidence: f32,
    pub event_id: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ResearchError {
    #[error("policy denied: {0}")]
    PolicyDenied(String),
    #[error("fetch error: {0}")]
    FetchError(String),
    #[error("inference error: {0}")]
    InferenceError(String),
    #[error("storage error: {0}")]
    StorageError(String),
}

/// Perform web research: fetch, summarize, store WebBrief.
pub async fn research(
    req: &ResearchRequest,
    policy: &PolicyEngine,
    backend: &Arc<dyn InferenceBackend>,
    cas: &CasStore,
    event_log: &mut EventLog,
    node_id: &str,
) -> std::result::Result<ResearchResult, ResearchError> {
    match policy.can_research_web(req.allow_web, req.redaction_required) {
        PolicyDecision::Allow => {}
        PolicyDecision::Deny(reason) => return Err(ResearchError::PolicyDenied(reason)),
    }

    let body = fetch_url(&req.url).await?;
    let summary = summarize(backend, &req.question, &body).await?;
    let sources = extract_sources(&body, &req.url);

    let artifact_id = uuid::Uuid::new_v4().to_string();

    let _content_ref = cas
        .put_bytes("text/plain", summary.as_bytes())
        .map_err(|e| ResearchError::StorageError(e.to_string()))?;

    let web_sources: Vec<WebSource> = sources
        .iter()
        .map(|url| WebSource {
            url: url.clone(),
            retrieved_at: None,
            publisher: String::new(),
            snippet: String::new(),
        })
        .collect();

    let event = EventEnvelope {
        event_id: artifact_id.clone(),
        r#type: EventType::WebBriefCreated as i32,
        node_id: Some(NodeId {
            value: node_id.into(),
        }),
        tenant_id: Some(TenantId {
            value: req.tenant_id.clone(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::WebBriefCreated(WebBriefCreated {
            artifact_id: artifact_id.clone(),
            question: req.question.clone(),
            summary: summary.clone(),
            sources: web_sources,
            confidence: 0.7,
            expires_unix_ms: 0,
        })),
        ..Default::default()
    };

    let stored = event_log
        .append(event)
        .map_err(|e| ResearchError::StorageError(e.to_string()))?;

    Ok(ResearchResult {
        artifact_id,
        question: req.question.clone(),
        summary,
        sources,
        confidence: 0.7,
        event_id: stored.event_id,
    })
}

async fn fetch_url(url: &str) -> std::result::Result<String, ResearchError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| ResearchError::FetchError(e.to_string()))?;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| ResearchError::FetchError(e.to_string()))?;

    resp.text()
        .await
        .map_err(|e| ResearchError::FetchError(e.to_string()))
}

async fn summarize(
    backend: &Arc<dyn InferenceBackend>,
    question: &str,
    body: &str,
) -> std::result::Result<String, ResearchError> {
    let truncated = if body.len() > 4000 {
        &body[..4000]
    } else {
        body
    };

    let prompt = format!(
        "Summarize the following web content to answer the question.\n\n\
         Question: {question}\n\n\
         Content:\n{truncated}\n\n\
         Provide a concise summary with key facts."
    );

    let req = node_ai::GenerateRequest {
        prompt,
        system: Some("You are a research assistant. Summarize web content concisely.".into()),
        max_tokens: 512,
        ..Default::default()
    };

    let resp = backend
        .generate(req)
        .await
        .map_err(|e| ResearchError::InferenceError(e.to_string()))?;

    Ok(resp.text)
}

fn extract_sources(html: &str, original_url: &str) -> Vec<String> {
    let mut sources = vec![original_url.to_string()];
    let mut search = html;
    while let Some(pos) = search.find("href=\"http") {
        let rest = &search[pos + 6..];
        if let Some(end) = rest.find('"') {
            let url = &rest[..end];
            if !sources.contains(&url.to_string()) {
                sources.push(url.to_string());
            }
        }
        search = &search[pos + 10..];
        if sources.len() >= 20 {
            break;
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_sources_works() {
        let html = r#"<a href="https://example.com">Link</a> <a href="http://test.org">Test</a>"#;
        let sources = extract_sources(html, "https://original.com");
        assert_eq!(sources.len(), 3);
        assert_eq!(sources[0], "https://original.com");
    }

    #[test]
    fn policy_denies_web_research() {
        let policy = PolicyEngine::with_defaults();
        let decision = policy.can_research_web(false, false);
        assert!(matches!(decision, PolicyDecision::Deny(_)));
    }
}
