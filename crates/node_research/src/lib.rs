//! Web research with citation extraction, WebBrief artifacts, and policy gating.
//!
//! Flow:
//! 1. Policy check: can_research_web()
//! 2. Fetch URL
//! 3. Extract/summarize with inference backend
//! 4. Store WebBrief as event
//! 5. Optionally redact before sharing

use std::path::Path;
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
    #[error("search error: {0}")]
    SearchError(String),
}

/// Perform a DuckDuckGo web search and return (title, url) pairs.
pub async fn web_search(
    query: &str,
    limit: usize,
) -> std::result::Result<Vec<(String, String)>, ResearchError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; rv:109.0) Gecko/20100101 Firefox/115.0")
        .build()
        .map_err(|e| ResearchError::SearchError(e.to_string()))?;

    let url = format!(
        "https://html.duckduckgo.com/html/?q={}",
        urlencoding::encode(query)
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| ResearchError::SearchError(e.to_string()))?;

    let html = resp
        .text()
        .await
        .map_err(|e| ResearchError::SearchError(e.to_string()))?;

    parse_ddg_html_results(&html, limit)
}

fn parse_ddg_html_results(
    html: &str,
    limit: usize,
) -> std::result::Result<Vec<(String, String)>, ResearchError> {
    let mut results = Vec::new();
    let mut pos = 0;
    while results.len() < limit {
        // DuckDuckGo HTML: links with class result__a, href may be uddg= encoded
        let link_start = match html[pos..].find("class=\"result__a\"") {
            Some(i) => pos + i,
            None => break,
        };
        let href_start = match html[link_start..].find("href=\"") {
            Some(i) => link_start + i + 6,
            None => break,
        };
        let href_end = match html[href_start..].find('"') {
            Some(i) => href_start + i,
            None => break,
        };
        let raw_url = html[href_start..href_end].trim();
        let url = if raw_url.contains("uddg=") {
            if let Some(uddg_start) = raw_url.find("uddg=") {
                let rest = &raw_url[uddg_start + 5..];
                let uddg_end = rest.find('&').unwrap_or(rest.len());
                let encoded = &rest[..uddg_end];
                match urlencoding::decode(encoded).map(|c| c.into_owned()) {
                    Ok(decoded) => decoded,
                    Err(_) => {
                        pos = href_end + 1;
                        continue;
                    }
                }
            } else {
                pos = href_end + 1;
                continue;
            }
        } else if raw_url.starts_with("https://duckduckgo.com/")
            || raw_url.starts_with("//duckduckgo.com")
        {
            pos = href_end + 1;
            continue;
        } else if raw_url.starts_with("//") {
            format!("https:{}", raw_url)
        } else {
            raw_url.to_string()
        };
        let title_start = html[href_end..]
            .find('>')
            .map(|i| href_end + i + 1)
            .unwrap_or(href_end);
        let title_end = match html[title_start..].find("</a>") {
            Some(i) => title_start + i,
            None => break,
        };
        let title = html[title_start..title_end]
            .replace("&#39;", "'")
            .replace("&amp;", "&")
            .replace("&quot;", "\"")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .trim()
            .to_string();
        if !title.is_empty() && !url.is_empty() && url.starts_with("http") {
            results.push((title, url));
        }
        pos = title_end + 4;
    }
    Ok(results)
}

/// Search the web and summarize the top result for chat context. Returns None if policy denies or search fails.
pub async fn search_and_summarize_for_chat(
    query: &str,
    policy: &PolicyEngine,
    backend: &Arc<dyn InferenceBackend>,
) -> Option<String> {
    search_and_summarize_with_details(query, policy, backend)
        .await
        .map(|r| r.0)
}

/// Like search_and_summarize_for_chat but returns (context, summary, source_url) for optional storage.
pub async fn search_and_summarize_with_details(
    query: &str,
    policy: &PolicyEngine,
    backend: &Arc<dyn InferenceBackend>,
) -> Option<(String, String, String)> {
    search_and_summarize_inner(query, policy, backend).await
}

/// Inner helper returning (context, summary, first_url) for optional storage.
async fn search_and_summarize_inner(
    query: &str,
    policy: &PolicyEngine,
    backend: &Arc<dyn InferenceBackend>,
) -> Option<(String, String, String)> {
    if !policy.can_research_web(true, true).is_allowed() {
        return None;
    }
    let results = web_search(query, 3).await.ok()?;
    let (_first_title, first_url) = results.first()?.clone();
    let body = fetch_url(&first_url).await.ok()?;
    let summary = summarize(backend, query, &body).await.ok()?;
    let mut out = format!("Web search result for \"{query}\":\n{summary}");
    if results.len() > 1 {
        out.push_str("\n\nOther results: ");
        out.push_str(
            &results[1..]
                .iter()
                .map(|(t, u)| format!("{t} ({u})"))
                .take(2)
                .collect::<Vec<_>>()
                .join("; "),
        );
    }
    Some((out, summary, first_url))
}

/// Store a web search result as a WebBrief in the knowledge base.
pub fn store_web_brief_from_search(
    query: &str,
    summary: &str,
    source_url: &str,
    cas: &CasStore,
    event_log: &mut EventLog,
    node_id: &str,
    db_path: &Path,
) -> bool {
    let sources = vec![WebSource {
        url: source_url.to_string(),
        retrieved_at: None,
        publisher: String::new(),
        snippet: String::new(),
    }];
    let artifact_id = format!("wb-{}", uuid::Uuid::new_v4());
    if cas.put_bytes("text/plain", summary.as_bytes()).is_err() {
        return false;
    }
    let event = EventEnvelope {
        event_id: artifact_id.clone(),
        r#type: EventType::WebBriefCreated as i32,
        node_id: Some(NodeId {
            value: node_id.into(),
        }),
        tenant_id: Some(TenantId {
            value: "public".into(),
        }),
        sensitivity: Sensitivity::Public as i32,
        payload: Some(event_envelope::Payload::WebBriefCreated(WebBriefCreated {
            artifact_id: artifact_id.clone(),
            question: query.into(),
            summary: summary.into(),
            sources,
            confidence: 0.7,
            expires_unix_ms: 0,
        })),
        ..Default::default()
    };
    let stored = match event_log.append(event) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let Ok(conn) = node_storage::sqlite_views::open_db(db_path) else {
        return false;
    };
    node_storage::projector::apply_event(&conn, &stored).is_ok()
}

/// Perform web research: fetch, summarize, store WebBrief.
/// If db_path is Some, projects the WebBrief event into SQLite views.
pub async fn research(
    req: &ResearchRequest,
    policy: &PolicyEngine,
    backend: &Arc<dyn InferenceBackend>,
    cas: &CasStore,
    event_log: &mut EventLog,
    node_id: &str,
    db_path: Option<&Path>,
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

    if let Some(path) = db_path {
        let conn = node_storage::sqlite_views::open_db(path)
            .map_err(|e| ResearchError::StorageError(e.to_string()))?;
        node_storage::projector::apply_event(&conn, &stored)
            .map_err(|e| ResearchError::StorageError(e.to_string()))?;
    }

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
        let end = body.floor_char_boundary(4000);
        &body[..end]
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
