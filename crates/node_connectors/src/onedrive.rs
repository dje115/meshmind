//! OneDrive connector: list and ingest files from Microsoft OneDrive via Graph API.
//!
//! Config from file (data/config/onedrive.json) or environment:
//! - MESHMIND_ONEDRIVE_CLIENT_ID
//! - MESHMIND_ONEDRIVE_TENANT_ID (e.g. "common" for multi-tenant)
//! - MESHMIND_ONEDRIVE_REFRESH_TOKEN
//! - MESHMIND_ONEDRIVE_CLIENT_SECRET (optional, for confidential clients)

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::{Connector, IngestBatchResult, IngestRow, SchemaColumn, TableInfo};

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

fn is_onedrive_path(path: &Path) -> bool {
    let s = path.display().to_string();
    s == "onedrive" || s.starts_with("onedrive://") || s.starts_with("onedrive:")
}

/// OneDrive OAuth configuration (persisted to data/config/onedrive.json).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OneDriveConfig {
    pub client_id: String,
    #[serde(default = "default_tenant")]
    pub tenant_id: String,
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
}

fn default_tenant() -> String {
    "common".to_string()
}

impl OneDriveConfig {
    pub fn is_empty(&self) -> bool {
        self.client_id.is_empty() || self.refresh_token.is_empty()
    }
}

/// Load OneDrive config from a JSON file. Returns None if file missing or invalid.
pub fn load_onedrive_config(path: &Path) -> Option<OneDriveConfig> {
    let text = std::fs::read_to_string(path).ok()?;
    let cfg: OneDriveConfig = serde_json::from_str(&text).ok()?;
    if cfg.is_empty() {
        return None;
    }
    Some(cfg)
}

/// Save OneDrive config to a JSON file.
pub fn save_onedrive_config(path: &Path, cfg: &OneDriveConfig) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create config dir")?;
    }
    let json = serde_json::to_string_pretty(cfg).context("serialize config")?;
    std::fs::write(path, json).context("write config file")?;
    Ok(())
}

fn onedrive_config_from_env() -> anyhow::Result<(String, String, String, Option<String>)> {
    let client_id = std::env::var("MESHMIND_ONEDRIVE_CLIENT_ID")
        .context("MESHMIND_ONEDRIVE_CLIENT_ID not set")?;
    let tenant_id =
        std::env::var("MESHMIND_ONEDRIVE_TENANT_ID").unwrap_or_else(|_| "common".to_string());
    let refresh_token = std::env::var("MESHMIND_ONEDRIVE_REFRESH_TOKEN")
        .context("MESHMIND_ONEDRIVE_REFRESH_TOKEN not set")?;
    let client_secret = std::env::var("MESHMIND_ONEDRIVE_CLIENT_SECRET").ok();
    Ok((client_id, tenant_id, refresh_token, client_secret))
}

fn refresh_access_token(
    client: &Client,
    client_id: &str,
    tenant: &str,
    refresh_token: &str,
    client_secret: Option<&str>,
) -> anyhow::Result<String> {
    let url = format!(
        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
        tenant
    );
    let mut form = vec![
        ("client_id", client_id),
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
    ];
    if let Some(secret) = client_secret {
        form.push(("client_secret", secret));
    }
    let resp = client
        .post(&url)
        .form(&form)
        .send()
        .context("token refresh request")?;
    let status = resp.status();
    let body = resp.text().context("token response body")?;
    if !status.is_success() {
        bail!("token refresh failed: {} - {}", status, body);
    }
    let json: serde_json::Value = serde_json::from_str(&body).context("parse token JSON")?;
    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .context("missing access_token in response")?;
    Ok(access_token.to_string())
}

#[derive(Deserialize)]
struct DriveItem {
    id: Option<String>,
    name: Option<String>,
    #[serde(rename = "size")]
    size_val: Option<i64>,
    #[serde(rename = "file")]
    file_info: Option<serde_json::Value>,
    #[serde(rename = "folder")]
    folder_info: Option<serde_json::Value>,
    #[serde(rename = "parentReference")]
    _parent_ref: Option<ParentRef>,
}

#[derive(Deserialize)]
struct ParentRef {
    #[serde(rename = "path")]
    _path: Option<String>,
}

#[derive(Deserialize)]
struct DriveItemResponse {
    value: Option<Vec<DriveItem>>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

pub struct OneDriveConnector {
    id: String,
    /// When Some, use this config instead of env vars.
    config: Option<OneDriveConfig>,
}

impl OneDriveConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            config: None,
        }
    }

    pub fn new_with_config(id: impl Into<String>, config: OneDriveConfig) -> Self {
        Self {
            id: id.into(),
            config: Some(config),
        }
    }

    fn get_config(&self) -> anyhow::Result<(String, String, String, Option<String>)> {
        if let Some(ref cfg) = self.config {
            if !cfg.is_empty() {
                return Ok((
                    cfg.client_id.clone(),
                    cfg.tenant_id.clone(),
                    cfg.refresh_token.clone(),
                    cfg.client_secret.clone(),
                ));
            }
        }
        onedrive_config_from_env()
    }
}

impl Connector for OneDriveConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        if !is_onedrive_path(path) {
            bail!("path is not a OneDrive path: {}", path.display());
        }
        let (client_id, tenant, refresh_token, client_secret) = self.get_config()?;
        let client = Client::builder().build().context("build HTTP client")?;
        let access_token = refresh_access_token(
            &client,
            &client_id,
            &tenant,
            &refresh_token,
            client_secret.as_deref(),
        )?;

        let url = format!("{}/me/drive/root/children?$top=200", GRAPH_BASE);
        let resp = client
            .get(&url)
            .bearer_auth(&access_token)
            .send()
            .context("list root children")?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().unwrap_or_default();
            bail!("Graph API error: {} - {}", status, body);
        }
        let data: DriveItemResponse = resp.json().context("parse drive items")?;
        let items = data.value.unwrap_or_default();

        let mut tables = Vec::new();
        let columns = vec![
            SchemaColumn {
                name: "id".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: true,
            },
            SchemaColumn {
                name: "name".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "web_url".into(),
                data_type: "TEXT".into(),
                nullable: true,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "size_bytes".into(),
                data_type: "INTEGER".into(),
                nullable: true,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "content_text".into(),
                data_type: "TEXT".into(),
                nullable: true,
                is_primary_key: false,
            },
        ];

        for item in &items {
            let name = item.name.as_deref().unwrap_or("unknown").to_string();
            if name.is_empty() || name == "unknown" {
                continue;
            }
            let is_folder = item.folder_info.is_some();
            let row_estimate = if is_folder { 100 } else { 1 };
            tables.push(TableInfo {
                table_name: name,
                columns: columns.clone(),
                row_count_estimate: row_estimate,
            });
        }

        if tables.is_empty() {
            tables.push(TableInfo {
                table_name: "root".to_string(),
                columns,
                row_count_estimate: 0,
            });
        }

        Ok(tables)
    }

    fn ingest_batch(
        &self,
        path: &Path,
        table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult> {
        if !is_onedrive_path(path) {
            bail!("path is not a OneDrive path: {}", path.display());
        }
        let (client_id, tenant, refresh_token, client_secret) = self.get_config()?;
        let client = Client::builder().build().context("build HTTP client")?;
        let access_token = refresh_access_token(
            &client,
            &client_id,
            &tenant,
            &refresh_token,
            client_secret.as_deref(),
        )?;

        let mut all_items: Vec<DriveItem> = Vec::new();
        let mut url = if table == "root" {
            format!("{}/me/drive/root/children?$top=999", GRAPH_BASE)
        } else {
            format!(
                "{}/me/drive/root:/{}:/children?$top=999",
                GRAPH_BASE,
                urlencoding::encode(table)
            )
        };

        loop {
            let resp = client
                .get(&url)
                .bearer_auth(&access_token)
                .send()
                .context("list drive items")?;
            if !resp.status().is_success() {
                let status = resp.status();
                if status.as_u16() == 429 {
                    if let Some(retry) = resp.headers().get("Retry-After") {
                        if let Ok(s) = retry.to_str() {
                            if let Ok(secs) = s.parse::<u64>() {
                                std::thread::sleep(std::time::Duration::from_secs(secs));
                                continue;
                            }
                        }
                    }
                }
                let body = resp.text().unwrap_or_default();
                bail!("Graph API error: {} - {}", status, body);
            }
            let data: DriveItemResponse = resp.json().context("parse drive items")?;
            let items = data.value.unwrap_or_default();
            all_items.extend(items);
            url = match data.next_link {
                Some(u) => u,
                None => break,
            };
        }

        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, all_items.len());
        let mut rows = Vec::new();

        const DOC_EXTS: &[&str] = &["pdf", "docx", "txt", "md", "rtf"];

        for (i, item) in all_items[start..end].iter().enumerate() {
            let entity_id = item.id.as_deref().unwrap_or("").to_string();
            let name = item.name.as_deref().unwrap_or("").to_string();
            let size = item.size_val.unwrap_or(0);
            let is_file = item.file_info.is_some();

            let mut columns = BTreeMap::new();
            columns.insert("id".into(), entity_id.clone());
            columns.insert("name".into(), name.clone());
            columns.insert("size_bytes".into(), size.to_string());
            columns.insert("web_url".into(), String::new());
            columns.insert("content_text".into(), String::new());

            if is_file && !entity_id.is_empty() {
                let ext = Path::new(&name)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if DOC_EXTS.contains(&ext.as_str()) && size > 0 && size < 10 * 1024 * 1024 {
                    let content_url =
                        format!("{}/me/drive/items/{}/content", GRAPH_BASE, entity_id);
                    match client.get(&content_url).bearer_auth(&access_token).send() {
                        Ok(r) if r.status().is_success() => {
                            if let Ok(bytes) = r.bytes() {
                                let text = extract_onedrive_text(&bytes, &ext);
                                columns.insert("content_text".into(), text);
                            }
                        }
                        _ => {}
                    }
                }
            }

            rows.push(IngestRow { entity_id, columns });
            debug!(idx = start + i, name = %name, "onedrive item");
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
        })
    }
}

const MAX_ONEDRIVE_TEXT_BYTES: usize = 100 * 1024;

fn extract_onedrive_text(bytes: &[u8], ext: &str) -> String {
    let text = match ext {
        "txt" | "md" | "rtf" => String::from_utf8_lossy(bytes).into_owned(),
        _ => String::new(),
    };
    if text.len() > MAX_ONEDRIVE_TEXT_BYTES {
        text.chars()
            .take(MAX_ONEDRIVE_TEXT_BYTES)
            .collect::<String>()
    } else {
        text
    }
}
