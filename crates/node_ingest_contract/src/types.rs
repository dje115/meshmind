//! Contract types for ingestion agent ↔ MeshMind core communication.
//!
//! Pipeline version: 1

use serde::{Deserialize, Serialize};

/// Contract version. Increment when breaking changes are made.
pub const PIPELINE_VERSION: u32 = 1;

/// Status of an ingested item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IngestItemStatus {
    Ingested,
    SkippedUnsupported,
    FailedExtraction,
    FailedOcr,
    FailedUnknown,
    Discovered,
    Unchanged,
    Queued,
    Processing,
    Deleted,
}

/// A single chunk of extracted content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestedChunk {
    /// Chunk index (0-based).
    pub chunk_index: u64,
    /// Extracted text for this chunk.
    pub chunk_text: String,
    /// Page number if applicable (1-based; 0 if N/A).
    #[serde(default)]
    pub page_number: u64,
}

/// Normalized item from an ingestion agent, ready for core to store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestedItem {
    /// MeshMind source identifier (approved source).
    pub source_id: String,
    /// Source type: filesystem, outlook, xero, sage, itquoter, onedrive, etc.
    pub source_type: String,
    /// Agent-specific item identifier (e.g. path, message_id, invoice_id).
    pub item_id: String,
    /// Human-readable display name.
    pub source_display_name: String,
    /// Human-readable origin label (e.g. "C:\\Quotes\\quote.docx").
    pub source_origin_label: String,
    /// Machine-readable location or lookup reference (path, id, key).
    pub source_locator: String,
    /// How to open the original: file://, outlook://, xero://, etc.
    pub source_open_target: String,
    /// Optional grouping context (watched folder, mailbox, tenant).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_parent: Option<String>,
    /// Path or external key (for filesystem: absolute path).
    pub path_or_external_key: String,
    /// Detected content type (mime or extension).
    pub content_type: String,
    /// Raw extracted text (concatenated or primary).
    pub extracted_text: String,
    /// Chunked content for indexing.
    pub chunks: Vec<IngestedChunk>,
    /// Extra metadata (JSON object).
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Whether OCR was attempted.
    pub ocr_attempted: bool,
    /// Whether OCR was used.
    pub ocr_used: bool,
    /// Extraction method used (e.g. "pdf_oxide", "tesseract", "docx-rust").
    pub extraction_method: String,
    /// Non-fatal warnings.
    #[serde(default)]
    pub warnings: Vec<String>,
    /// Final ingest status.
    pub ingest_status: IngestItemStatus,
    /// Failure reason when status indicates failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    /// Source item modified timestamp (Unix ms).
    pub source_modified_at: i64,
    /// When this item was ingested (Unix ms).
    pub ingested_at: i64,
    /// Content hash (SHA-256 hex) for deduplication.
    pub content_hash: String,
    /// Contract/pipeline version.
    pub pipeline_version: u32,
}

/// Ingest job summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestJob {
    pub job_id: String,
    pub source_id: String,
    pub started_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<i64>,
    pub status: String,
    pub counts: IngestJobCounts,
}

/// Counts for an ingest job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestJobCounts {
    pub seen: u64,
    pub ingested: u64,
    pub skipped: u64,
    pub failed: u64,
}

/// Source watch configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceWatch {
    pub source_id: String,
    pub source_kind: String,
    /// Root path or endpoint (for filesystem: folder path).
    pub root: String,
    pub enabled: bool,
    /// Polling or watch mode.
    pub mode: String,
    /// Include patterns (glob).
    #[serde(default)]
    pub include_patterns: Vec<String>,
    /// Exclude patterns (glob).
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
    /// Recursive folder scan.
    #[serde(default = "default_true")]
    pub recursion: bool,
    /// Change detection rules (e.g. mtime, hash).
    #[serde(default)]
    pub change_detection: Vec<String>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ingested_item_roundtrip() {
        let item = IngestedItem {
            source_id: "src-1".into(),
            source_type: "filesystem".into(),
            item_id: "C:/docs/report.pdf".into(),
            source_display_name: "report.pdf".into(),
            source_origin_label: "C:\\docs\\report.pdf".into(),
            source_locator: "C:/docs/report.pdf".into(),
            source_open_target: "file:///C:/docs/report.pdf".into(),
            source_parent: Some("C:/docs".into()),
            path_or_external_key: "C:/docs/report.pdf".into(),
            content_type: "application/pdf".into(),
            extracted_text: "Hello world".into(),
            chunks: vec![IngestedChunk {
                chunk_index: 0,
                chunk_text: "Hello world".into(),
                page_number: 1,
            }],
            metadata: serde_json::json!({"file_size": 1234}),
            ocr_attempted: false,
            ocr_used: false,
            extraction_method: "pdf_oxide".into(),
            warnings: vec![],
            ingest_status: IngestItemStatus::Ingested,
            failure_reason: None,
            source_modified_at: 1700000000000,
            ingested_at: 1700000001000,
            content_hash: "abc123".into(),
            pipeline_version: 1,
        };
        let json = serde_json::to_string(&item).unwrap();
        let decoded: IngestedItem = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.source_id, item.source_id);
        assert_eq!(decoded.source_open_target, item.source_open_target);
        assert_eq!(decoded.chunks.len(), 1);
    }
}
