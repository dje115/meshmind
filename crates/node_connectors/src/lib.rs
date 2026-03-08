//! Connector trait and implementations for data source introspection and ingestion.
//!
//! Connectors:
//! - SQLiteConnector: discover/inspect/ingest from SQLite databases
//! - CsvFolderConnector: discover/inspect/ingest from CSV directories
//! - JsonFolderConnector: discover/inspect/ingest from JSON directories
//! - ImageConnector: extract EXIF/GPS metadata from image folders
//! - DocumentConnector: extract text from PDF, DOCX, TXT, Markdown folders (legacy path;
//!   prefer the filesystem ingestion agent when available for folder walking, extraction,
//!   OCR, and provenance; agent POSTs to /v1/ingest/items)

mod pdf_ocr;

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;
use walkdir::WalkDir;

use anyhow::{bail, Context};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use tracing::debug;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SchemaColumn {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
    pub is_primary_key: bool,
}

#[derive(Debug, Clone)]
pub struct TableInfo {
    pub table_name: String,
    pub columns: Vec<SchemaColumn>,
    pub row_count_estimate: u64,
}

#[derive(Debug, Clone)]
pub struct IngestRow {
    pub entity_id: String,
    pub columns: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct IngestBatchResult {
    pub table_name: String,
    pub rows: Vec<IngestRow>,
    pub offset: u64,
    /// Per-file ingest results (DocumentConnector only). Populated when offset == 0.
    #[allow(clippy::module_name_repetitions)]
    pub file_results: Option<Vec<FileIngestResult>>,
}

/// Status of processing a single file during document folder ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileIngestStatus {
    Ingested,
    SkippedUnsupported,
    FailedExtraction,
    FailedOcr,
    FailedUnknown,
}

/// Per-file result from document folder ingestion.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileIngestResult {
    pub filename: String,
    pub file_path: String,
    pub detected_type: String,
    pub status: FileIngestStatus,
    pub failure_reason: Option<String>,
    pub ocr_attempted: bool,
    pub chunks_created: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnClassResult {
    pub classification: String,
    pub is_pii: bool,
    pub is_secret: bool,
    pub suggested_sensitivity: i32,
}

// ── Connector trait ─────────────────────────────────────────────────────────

pub trait Connector: Send + Sync {
    fn id(&self) -> &str;
    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>>;
    fn ingest_batch(
        &self,
        path: &Path,
        table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult>;
}

// ── SQLiteConnector ─────────────────────────────────────────────────────────

pub struct SQLiteConnector {
    id: String,
}

impl SQLiteConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

fn sqlite_value_to_string(v: ValueRef<'_>) -> String {
    match v {
        ValueRef::Null => String::new(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Text(b) => String::from_utf8_lossy(b).into_owned(),
        ValueRef::Blob(b) => format!("<{} bytes>", b.len()),
    }
}

impl Connector for SQLiteConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let conn = Connection::open(path).context("open SQLite database")?;
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        )?;
        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .collect::<Result<Vec<_>, _>>()?;

        let mut tables = Vec::new();
        for table_name in table_names {
            debug!(table = %table_name, "inspecting SQLite table");

            let mut col_stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table_name))?;
            let columns: Vec<SchemaColumn> = col_stmt
                .query_map([], |row| {
                    let notnull: i32 = row.get(3)?;
                    let pk: i32 = row.get(5)?;
                    Ok(SchemaColumn {
                        name: row.get(1)?,
                        data_type: row.get::<_, String>(2).unwrap_or_default(),
                        nullable: notnull == 0,
                        is_primary_key: pk != 0,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let count: i64 = conn.query_row(
                &format!("SELECT COUNT(*) FROM \"{}\"", table_name),
                [],
                |row| row.get(0),
            )?;

            tables.push(TableInfo {
                table_name,
                columns,
                row_count_estimate: count as u64,
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
        let conn = Connection::open(path).context("open SQLite database")?;
        let sql = format!(
            "SELECT * FROM \"{}\" LIMIT {} OFFSET {}",
            table, limit, offset
        );
        let mut stmt = conn.prepare(&sql)?;
        let col_names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();

        let mut rows = Vec::new();
        let mut qr = stmt.query([])?;
        let mut row_idx = 0u64;
        while let Some(row) = qr.next()? {
            let entity_id = format!("{}", offset + row_idx);
            let mut columns = BTreeMap::new();
            for (i, name) in col_names.iter().enumerate() {
                columns.insert(name.clone(), sqlite_value_to_string(row.get_ref(i)?));
            }
            rows.push(IngestRow { entity_id, columns });
            row_idx += 1;
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
            file_results: None,
        })
    }
}

// ── CsvFolderConnector ──────────────────────────────────────────────────────

pub struct CsvFolderConnector {
    id: String,
}

impl CsvFolderConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();

    while let Some(c) = chars.next() {
        if in_quotes {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(c);
            }
        } else if c == '"' {
            in_quotes = true;
        } else if c == ',' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    fields.push(current);
    fields
}

impl Connector for CsvFolderConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let entries = fs::read_dir(path).context("read CSV directory")?;
        let mut tables = Vec::new();

        for entry in entries {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("csv") {
                continue;
            }
            let table_name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            debug!(table = %table_name, "inspecting CSV file");

            let file = fs::File::open(&file_path)?;
            let reader = BufReader::new(file);
            let mut lines = reader.lines();

            let headers = match lines.next() {
                Some(Ok(line)) => parse_csv_line(&line),
                _ => continue,
            };

            let row_count = lines
                .filter(|l| l.as_ref().map(|s| !s.trim().is_empty()).unwrap_or(false))
                .count() as u64;

            let columns = headers
                .into_iter()
                .map(|h| SchemaColumn {
                    name: h.trim().to_string(),
                    data_type: "TEXT".to_string(),
                    nullable: true,
                    is_primary_key: false,
                })
                .collect();

            tables.push(TableInfo {
                table_name,
                columns,
                row_count_estimate: row_count,
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
        let file_path = path.join(format!("{}.csv", table));
        if !file_path.exists() {
            bail!("CSV file not found: {}", file_path.display());
        }

        let file = fs::File::open(&file_path)?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let headers = match lines.next() {
            Some(Ok(line)) => parse_csv_line(&line),
            _ => bail!("CSV file is empty or has no header"),
        };

        let mut rows = Vec::new();
        let mut effective_idx = 0u64;

        for line_result in lines {
            let line = line_result?;
            if line.trim().is_empty() {
                continue;
            }
            if effective_idx < offset {
                effective_idx += 1;
                continue;
            }
            if effective_idx >= offset + limit {
                break;
            }
            let fields = parse_csv_line(&line);
            let mut columns = BTreeMap::new();
            for (i, header) in headers.iter().enumerate() {
                columns.insert(
                    header.trim().to_string(),
                    fields.get(i).cloned().unwrap_or_default(),
                );
            }
            rows.push(IngestRow {
                entity_id: format!("{}", effective_idx),
                columns,
            });
            effective_idx += 1;
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
            file_results: None,
        })
    }
}

// ── JsonFolderConnector ─────────────────────────────────────────────────────

pub struct JsonFolderConnector {
    id: String,
}

impl JsonFolderConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

fn read_json_objects(path: &Path) -> anyhow::Result<Vec<serde_json::Value>> {
    let content = fs::read_to_string(path).context("read JSON file")?;
    let trimmed = content.trim();

    if trimmed.starts_with('[') {
        let arr: Vec<serde_json::Value> = serde_json::from_str(trimmed)?;
        return Ok(arr);
    }

    let mut objects = Vec::new();
    for line in trimmed.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        objects.push(serde_json::from_str(line)?);
    }
    Ok(objects)
}

impl Connector for JsonFolderConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let entries = fs::read_dir(path).context("read JSON directory")?;
        let mut tables = Vec::new();

        for entry in entries {
            let entry = entry?;
            let file_path = entry.path();
            if file_path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let table_name = file_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            debug!(table = %table_name, "inspecting JSON file");

            let objects = read_json_objects(&file_path)?;
            let row_count = objects.len() as u64;

            let columns = if let Some(first) = objects.first() {
                if let Some(obj) = first.as_object() {
                    obj.keys()
                        .map(|k| SchemaColumn {
                            name: k.clone(),
                            data_type: "JSON".to_string(),
                            nullable: true,
                            is_primary_key: false,
                        })
                        .collect()
                } else {
                    vec![]
                }
            } else {
                vec![]
            };

            tables.push(TableInfo {
                table_name,
                columns,
                row_count_estimate: row_count,
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
        let file_path = path.join(format!("{}.json", table));
        if !file_path.exists() {
            bail!("JSON file not found: {}", file_path.display());
        }

        let objects = read_json_objects(&file_path)?;
        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, objects.len());
        let mut rows = Vec::new();

        for (i, obj) in objects[start..end].iter().enumerate() {
            let mut columns = BTreeMap::new();
            if let Some(map) = obj.as_object() {
                for (k, v) in map {
                    let val = match v {
                        serde_json::Value::String(s) => s.clone(),
                        serde_json::Value::Null => String::new(),
                        other => other.to_string(),
                    };
                    columns.insert(k.clone(), val);
                }
            }
            rows.push(IngestRow {
                entity_id: format!("{}", start + i),
                columns,
            });
        }

        Ok(IngestBatchResult {
            table_name: table.to_string(),
            rows,
            offset,
            file_results: None,
        })
    }
}

// ── ImageConnector ──────────────────────────────────────────────────────────

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "tiff", "tif", "heic", "heif", "webp"];

fn is_image_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            IMAGE_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

const IMAGE_SCHEMA_COLUMNS: &[&str] = &[
    "filename",
    "file_path",
    "file_size_bytes",
    "gps_latitude",
    "gps_longitude",
    "gps_altitude",
    "date_taken",
    "camera_make",
    "camera_model",
    "image_width",
    "image_height",
    "orientation",
    "exposure_time",
    "f_number",
    "iso_speed",
];

fn dms_to_decimal(dms: &exif::Value, ref_val: &str) -> Option<f64> {
    if let exif::Value::Rational(ref rationals) = dms {
        if rationals.len() >= 3 {
            let d = rationals[0].num as f64 / rationals[0].denom as f64;
            let m = rationals[1].num as f64 / rationals[1].denom as f64;
            let s = rationals[2].num as f64 / rationals[2].denom as f64;
            let decimal = d + m / 60.0 + s / 3600.0;
            return Some(if ref_val == "S" || ref_val == "W" {
                -decimal
            } else {
                decimal
            });
        }
    }
    None
}

fn exif_field_string(exif_data: &exif::Exif, tag: exif::Tag) -> String {
    exif_data
        .get_field(tag, exif::In::PRIMARY)
        .map(|f| f.display_value().to_string())
        .unwrap_or_default()
}

fn extract_image_metadata(path: &Path) -> BTreeMap<String, String> {
    let mut cols = BTreeMap::new();
    cols.insert(
        "filename".into(),
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string(),
    );
    cols.insert("file_path".into(), path.to_string_lossy().into_owned());
    cols.insert(
        "file_size_bytes".into(),
        fs::metadata(path)
            .map(|m| m.len().to_string())
            .unwrap_or_default(),
    );

    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return cols,
    };
    let mut reader = std::io::BufReader::new(file);
    let exif_data = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(e) => e,
        Err(_) => return cols,
    };

    let lat_ref = exif_field_string(&exif_data, exif::Tag::GPSLatitudeRef);
    let lon_ref = exif_field_string(&exif_data, exif::Tag::GPSLongitudeRef);

    if let Some(f) = exif_data.get_field(exif::Tag::GPSLatitude, exif::In::PRIMARY) {
        if let Some(dec) = dms_to_decimal(&f.value, &lat_ref) {
            cols.insert("gps_latitude".into(), format!("{:.6}", dec));
        }
    }
    if let Some(f) = exif_data.get_field(exif::Tag::GPSLongitude, exif::In::PRIMARY) {
        if let Some(dec) = dms_to_decimal(&f.value, &lon_ref) {
            cols.insert("gps_longitude".into(), format!("{:.6}", dec));
        }
    }
    if let Some(f) = exif_data.get_field(exif::Tag::GPSAltitude, exif::In::PRIMARY) {
        cols.insert("gps_altitude".into(), f.display_value().to_string());
    }

    cols.insert(
        "date_taken".into(),
        exif_field_string(&exif_data, exif::Tag::DateTimeOriginal),
    );
    cols.insert(
        "camera_make".into(),
        exif_field_string(&exif_data, exif::Tag::Make),
    );
    cols.insert(
        "camera_model".into(),
        exif_field_string(&exif_data, exif::Tag::Model),
    );
    cols.insert(
        "image_width".into(),
        exif_field_string(&exif_data, exif::Tag::PixelXDimension),
    );
    cols.insert(
        "image_height".into(),
        exif_field_string(&exif_data, exif::Tag::PixelYDimension),
    );
    cols.insert(
        "orientation".into(),
        exif_field_string(&exif_data, exif::Tag::Orientation),
    );
    cols.insert(
        "exposure_time".into(),
        exif_field_string(&exif_data, exif::Tag::ExposureTime),
    );
    cols.insert(
        "f_number".into(),
        exif_field_string(&exif_data, exif::Tag::FNumber),
    );
    cols.insert(
        "iso_speed".into(),
        exif_field_string(&exif_data, exif::Tag::PhotographicSensitivity),
    );

    cols
}

pub struct ImageConnector {
    id: String,
}

impl ImageConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl Connector for ImageConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let entries = fs::read_dir(path).context("read image directory")?;
        let mut file_count = 0u64;

        for entry in entries.flatten() {
            if entry.path().is_file() && is_image_file(&entry.path()) {
                file_count += 1;
            }
        }

        let columns = IMAGE_SCHEMA_COLUMNS
            .iter()
            .map(|name| SchemaColumn {
                name: name.to_string(),
                data_type: "TEXT".to_string(),
                nullable: true,
                is_primary_key: *name == "filename",
            })
            .collect();

        Ok(vec![TableInfo {
            table_name: "images".to_string(),
            columns,
            row_count_estimate: file_count,
        }])
    }

    fn ingest_batch(
        &self,
        path: &Path,
        _table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult> {
        let mut image_files: Vec<_> = fs::read_dir(path)
            .context("read image directory")?
            .flatten()
            .filter(|e| e.path().is_file() && is_image_file(&e.path()))
            .collect();
        image_files.sort_by_key(|e| e.file_name());

        let start = offset as usize;
        let end = std::cmp::min(start + limit as usize, image_files.len());
        let mut rows = Vec::new();

        for (i, entry) in image_files[start..end].iter().enumerate() {
            let columns = extract_image_metadata(&entry.path());
            rows.push(IngestRow {
                entity_id: format!("{}", start + i),
                columns,
            });
        }

        Ok(IngestBatchResult {
            table_name: "images".to_string(),
            rows,
            offset,
            file_results: None,
        })
    }
}

// ── DocumentConnector ───────────────────────────────────────────────────────

/// Supported document formats (ingestion + extraction).
pub const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "doc", "xls", "xlsx", "pptx", "ppt", "txt", "md", "rtf",
];
/// Known unsupported formats (reported as skipped_unsupported, not silently ignored).
/// Visio (.vsd, .vsdx) requires external tooling; no mature Rust parser exists.
pub const UNSUPPORTED_EXTENSIONS: &[&str] = &["vsd", "vsdx"];

const MAX_DOCUMENT_TEXT_BYTES: usize = 100 * 1024;

/// Chunk size in characters for document splitting.
pub const CHUNK_SIZE: usize = 1500;
/// Overlap in characters between consecutive chunks.
pub const CHUNK_OVERLAP: usize = 200;

/// Split text into chunks with overlap. Small texts return a single chunk.
fn chunk_text(text: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() {
        return vec![];
    }
    if text.len() <= chunk_size {
        return vec![text.to_string()];
    }
    let step = chunk_size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = (start + chunk_size).min(text.len());
        let chunk = &text[start..end];
        let chunk_trimmed = chunk.trim();
        if !chunk_trimmed.is_empty() {
            chunks.push(chunk_trimmed.to_string());
        }
        if end >= text.len() {
            break;
        }
        start += step;
    }
    chunks
}

fn is_document_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            DOCUMENT_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

fn is_unsupported_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            let lower = e.to_ascii_lowercase();
            UNSUPPORTED_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false)
}

/// Result of extracting text from a document file.
struct ExtractResult {
    text: String,
    file_type: String,
    page_count: u64,
    /// True when OCR was used (scanned PDF fallback).
    ocr_used: bool,
    /// True if OCR was attempted (e.g. scanned PDF) but failed or yielded nothing.
    ocr_attempted: bool,
    /// Failure reason when extraction yielded no usable text.
    failure_reason: Option<String>,
}

/// Extract text from PDF using pdf_oxide. If extracted text is very short (< 200 chars),
/// treat as scanned and run OCR fallback (pdftoppm + tesseract).
fn extract_pdf_text(path: &Path) -> ExtractResult {
    let mut doc = match pdf_oxide::PdfDocument::open(path) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "PDF open failed");
            return ExtractResult {
                text: String::new(),
                file_type: "pdf".into(),
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
                failure_reason: Some(format!("PDF open failed: {e}")),
            };
        }
    };
    let page_count = doc.page_count().unwrap_or(0) as u64;
    let mut text = String::new();
    for page in 0..(page_count as usize) {
        match doc.extract_text(page) {
            Ok(page_text) => {
                if !page_text.is_empty() {
                    if !text.is_empty() {
                        text.push_str("\n\n");
                    }
                    text.push_str(&page_text);
                }
            }
            Err(e) => {
                tracing::debug!(path = %path.display(), page = page, error = %e, "PDF page extract failed");
            }
        }
    }

    let mut ocr_attempted = false;

    // OCR fallback for scanned PDFs (little or no extractable text)
    if text.trim().len() < pdf_ocr::SCANNED_PDF_TEXT_THRESHOLD {
        match pdf_ocr::run_pdf_ocr(path) {
            Some(ocr) if !ocr.text.trim().is_empty() => {
                tracing::info!(path = %path.display(), pages = ocr.page_count, "OCR fallback recovered text from scanned PDF");
                let truncated = if ocr.text.len() > MAX_DOCUMENT_TEXT_BYTES {
                    ocr.text[..MAX_DOCUMENT_TEXT_BYTES].to_string()
                } else {
                    ocr.text
                };
                return ExtractResult {
                    text: truncated,
                    file_type: "pdf".into(),
                    page_count: ocr.page_count,
                    ocr_used: true,
                    ocr_attempted: true,
                    failure_reason: None,
                };
            }
            Some(_) => {
                return ExtractResult {
                    text: String::new(),
                    file_type: "pdf".into(),
                    page_count,
                    ocr_used: false,
                    ocr_attempted: true,
                    failure_reason: Some("OCR ran but yielded no text".into()),
                };
            }
            None => {
                // OCR tools not available - use whatever pdf_oxide extracted (even if short)
                ocr_attempted = true;
                if text.trim().is_empty() {
                    return ExtractResult {
                        text: String::new(),
                        file_type: "pdf".into(),
                        page_count,
                        ocr_used: false,
                        ocr_attempted: true,
                        failure_reason: Some(
                            "OCR failed (pdftoppm/tesseract not available or error)".into(),
                        ),
                    };
                }
                // Has some text from pdf_oxide - use it; don't fail just because OCR isn't installed
            }
        }
    }

    let truncated = if text.len() > MAX_DOCUMENT_TEXT_BYTES {
        text[..MAX_DOCUMENT_TEXT_BYTES].to_string()
    } else {
        text
    };
    ExtractResult {
        text: truncated,
        file_type: "pdf".into(),
        page_count,
        ocr_used: false,
        ocr_attempted,
        failure_reason: None,
    }
}

fn extract_text_from_file(path: &Path) -> ExtractResult {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let file_type = ext.clone();

    let mut result = match ext.as_str() {
        "txt" | "md" | "rtf" => {
            let text = fs::read_to_string(path).unwrap_or_default();
            ExtractResult {
                failure_reason: if text.is_empty() {
                    Some("File empty or read failed".into())
                } else {
                    None
                },
                text,
                file_type,
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
            }
        }
        "pdf" => extract_pdf_text(path),
        "docx" => {
            let text = match docx_rust::DocxFile::from_file(path) {
                Ok(docx_file) => match docx_file.parse() {
                    Ok(docx) => extract_docx_text(&docx.document),
                    Err(e) => {
                        return ExtractResult {
                            text: String::new(),
                            file_type,
                            page_count: 0,
                            ocr_used: false,
                            ocr_attempted: false,
                            failure_reason: Some(format!("DOCX parse failed: {e}")),
                        };
                    }
                },
                Err(e) => {
                    return ExtractResult {
                        text: String::new(),
                        file_type,
                        page_count: 0,
                        ocr_used: false,
                        ocr_attempted: false,
                        failure_reason: Some(format!("DOCX open failed: {e}")),
                    };
                }
            };
            let failure_reason = if text.is_empty() {
                Some("DOCX yielded no text".into())
            } else {
                None
            };
            ExtractResult {
                text,
                file_type,
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
                failure_reason,
            }
        }
        "xls" | "xlsx" => extract_excel_text(path, &file_type),
        "pptx" => extract_pptx_text(path, &file_type),
        "doc" => extract_legacy_word_text(path, &file_type),
        "ppt" => extract_legacy_ppt_text(path, &file_type),
        _ => ExtractResult {
            text: String::new(),
            file_type,
            page_count: 0,
            ocr_used: false,
            ocr_attempted: false,
            failure_reason: Some(format!("Unsupported format: {ext}")),
        },
    };

    let truncated = if result.text.len() > MAX_DOCUMENT_TEXT_BYTES {
        result.text[..MAX_DOCUMENT_TEXT_BYTES].to_string()
    } else {
        result.text
    };
    result.text = truncated;
    result
}

fn extract_docx_text(docx: &docx_rust::document::Document) -> String {
    let mut text = String::new();
    for child in &docx.body.content {
        if let docx_rust::document::BodyContent::Paragraph(para) = child {
            for content in &para.content {
                if let docx_rust::document::ParagraphContent::Run(run) = content {
                    for rc in &run.content {
                        if let docx_rust::document::RunContent::Text(t) = rc {
                            text.push_str(&t.text);
                        }
                    }
                }
            }
            text.push('\n');
        }
    }
    text
}

/// Extract text from Excel (.xls, .xlsx) using litchi.
/// Litchi supports both legacy XLS (OLE2) and XLSX (OOXML) with better compatibility
/// than calamine, which panics on some valid XLS files.
fn extract_excel_text(path: &Path, file_type: &str) -> ExtractResult {
    use litchi::sheet::{CellValue, Workbook};

    let workbook: Box<dyn Workbook> = if file_type == "xls" {
        match litchi::sheet::open_xls_workbook(path) {
            Ok(wb) => Box::new(wb),
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "XLS open failed");
                return ExtractResult {
                    text: String::new(),
                    file_type: file_type.to_string(),
                    page_count: 0,
                    ocr_used: false,
                    ocr_attempted: false,
                    failure_reason: Some(format!("XLS open failed: {e}")),
                };
            }
        }
    } else {
        match litchi::sheet::open_workbook(path) {
            Ok(wb) => wb,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "Excel open failed");
                return ExtractResult {
                    text: String::new(),
                    file_type: file_type.to_string(),
                    page_count: 0,
                    ocr_used: false,
                    ocr_attempted: false,
                    failure_reason: Some(format!("Excel open failed: {e}")),
                };
            }
        }
    };

    let mut text = String::new();
    for name in workbook.worksheet_names() {
        if let Ok(ws) = workbook.worksheet_by_name(&name) {
            for row_idx in 0..ws.row_count() {
                if let Ok(row_vals) = ws.row(row_idx) {
                    let row_text: Vec<String> = row_vals
                        .iter()
                        .map(|cv| match cv {
                            CellValue::Empty => String::new(),
                            CellValue::Bool(b) => b.to_string(),
                            CellValue::Int(n) => n.to_string(),
                            CellValue::Float(f) => f.to_string(),
                            CellValue::String(s) => s.clone(),
                            CellValue::DateTime(d) => d.to_string(),
                            CellValue::Error(e) => e.clone(),
                        })
                        .filter(|s| !s.is_empty())
                        .collect();
                    if !row_text.is_empty() {
                        text.push_str(&row_text.join("\t"));
                        text.push('\n');
                    }
                }
            }
        }
    }
    let failure_reason = if text.trim().is_empty() {
        Some("Excel yielded no text".into())
    } else {
        None
    };
    ExtractResult {
        text,
        file_type: file_type.to_string(),
        page_count: 0,
        ocr_used: false,
        ocr_attempted: false,
        failure_reason,
    }
}

/// Extract text from PowerPoint (.pptx) using undoc.
fn extract_pptx_text(path: &Path, file_type: &str) -> ExtractResult {
    use undoc::render;
    let doc = match undoc::parse_file(path) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "PPTX parse failed");
            return ExtractResult {
                text: String::new(),
                file_type: file_type.to_string(),
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
                failure_reason: Some(format!("PPTX parse failed: {e}")),
            };
        }
    };
    let options = render::RenderOptions::default();
    let text = match render::to_text(&doc, &options) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "PPTX render failed");
            return ExtractResult {
                text: String::new(),
                file_type: file_type.to_string(),
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
                failure_reason: Some(format!("PPTX render failed: {e}")),
            };
        }
    };
    let failure_reason = if text.trim().is_empty() {
        Some("PPTX yielded no text".into())
    } else {
        None
    };
    ExtractResult {
        text,
        file_type: file_type.to_string(),
        page_count: 0,
        ocr_used: false,
        ocr_attempted: false,
        failure_reason,
    }
}

/// Extract text from legacy Word (.doc) using litchi, with RTF fallback.
/// Some .doc files are RTF-in-disguise (e.g. "Save as Word 97" producing RTF);
/// when litchi fails with "Not a valid Office file", we try parsing as RTF.
fn extract_legacy_word_text(path: &Path, file_type: &str) -> ExtractResult {
    use litchi::Document;

    // Try litchi first (OLE2 / OOXML)
    if let Ok(doc) = Document::open(path) {
        if let Ok(text) = doc.text() {
            if !text.trim().is_empty() {
                return ExtractResult {
                    text,
                    file_type: file_type.to_string(),
                    page_count: 0,
                    ocr_used: false,
                    ocr_attempted: false,
                    failure_reason: None,
                };
            }
        }
    }

    // Fallback: try RTF (many .doc files are actually RTF)
    if let Ok(bytes) = fs::read(path) {
        let starts_rtf = bytes.len() >= 6 && &bytes[..6] == b"{\\rtf1";
        let starts_rtf_ansi = bytes.len() >= 11 && &bytes[..11] == b"{\\rtf1\\ansi";
        if starts_rtf || starts_rtf_ansi {
            if let Ok(s) = std::str::from_utf8(&bytes) {
                match rtf_extract_text(s) {
                    Ok(text) if !text.trim().is_empty() => {
                        return ExtractResult {
                            text,
                            file_type: file_type.to_string(),
                            page_count: 0,
                            ocr_used: false,
                            ocr_attempted: false,
                            failure_reason: None,
                        };
                    }
                    _ => {}
                }
            }
        }
    }

    tracing::warn!(path = %path.display(), "DOC parse failed (tried litchi OLE2/OOXML and RTF fallback)");
    ExtractResult {
        text: String::new(),
        file_type: file_type.to_string(),
        page_count: 0,
        ocr_used: false,
        ocr_attempted: false,
        failure_reason: Some("DOC parse failed: not a valid Office or RTF file".into()),
    }
}

fn rtf_extract_text(rtf: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    use rtf_parser::lexer::Lexer;
    use rtf_parser::parser::Parser;

    let tokens = Lexer::scan(rtf)?;
    let doc = Parser::new(tokens).parse()?;
    Ok(doc.get_text())
}

/// Extract text from legacy PowerPoint (.ppt) using litchi.
fn extract_legacy_ppt_text(path: &Path, file_type: &str) -> ExtractResult {
    use litchi::Presentation;
    let pres = match Presentation::open(path) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "PPT parse failed");
            return ExtractResult {
                text: String::new(),
                file_type: file_type.to_string(),
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
                failure_reason: Some(format!("PPT parse failed: {e}")),
            };
        }
    };
    let text = match pres.text() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "PPT text extraction failed");
            return ExtractResult {
                text: String::new(),
                file_type: file_type.to_string(),
                page_count: 0,
                ocr_used: false,
                ocr_attempted: false,
                failure_reason: Some(format!("PPT text extraction failed: {e}")),
            };
        }
    };
    let failure_reason = if text.trim().is_empty() {
        Some("PPT yielded no text".into())
    } else {
        None
    };
    ExtractResult {
        text,
        file_type: file_type.to_string(),
        page_count: 0,
        ocr_used: false,
        ocr_attempted: false,
        failure_reason,
    }
}

pub struct DocumentConnector {
    id: String,
}

impl DocumentConnector {
    pub fn new(id: impl Into<String>) -> Self {
        Self { id: id.into() }
    }
}

impl Connector for DocumentConnector {
    fn id(&self) -> &str {
        &self.id
    }

    fn inspect_schema(&self, path: &Path) -> anyhow::Result<Vec<TableInfo>> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let file_count = WalkDir::new(&path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .filter(|e| is_document_file(e.path()) || is_unsupported_file(e.path()))
            .count() as u64;

        let columns = vec![
            SchemaColumn {
                name: "document_id".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "chunk_index".into(),
                data_type: "INTEGER".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "chunk_text".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "source_file".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "page_number".into(),
                data_type: "INTEGER".into(),
                nullable: true,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "filename".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: true,
            },
            SchemaColumn {
                name: "file_path".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "file_type".into(),
                data_type: "TEXT".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "file_size_bytes".into(),
                data_type: "INTEGER".into(),
                nullable: false,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "content_text".into(),
                data_type: "TEXT".into(),
                nullable: true,
                is_primary_key: false,
            },
            SchemaColumn {
                name: "ocr_used".into(),
                data_type: "INTEGER".into(),
                nullable: true,
                is_primary_key: false,
            },
        ];

        Ok(vec![TableInfo {
            table_name: "documents".to_string(),
            columns,
            row_count_estimate: file_count * 3, // rough estimate: avg 3 chunks per doc
        }])
    }

    fn ingest_batch(
        &self,
        path: &Path,
        _table: &str,
        offset: u64,
        limit: u64,
    ) -> anyhow::Result<IngestBatchResult> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let base_path = path.as_path();

        // Recursively collect all document and unsupported files
        let mut all_files: Vec<std::path::PathBuf> = WalkDir::new(&path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_path_buf())
            .filter(|p| is_document_file(p) || is_unsupported_file(p))
            .collect();
        all_files.sort();

        let mut all_chunks: Vec<IngestRow> = Vec::new();
        let mut file_results: Vec<FileIngestResult> = Vec::new();

        for fpath in &all_files {
            let filename = fpath
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();
            let file_path = fpath.to_string_lossy().into_owned();
            let file_size = fs::metadata(fpath).map(|m| m.len()).unwrap_or(0);

            // Relative path for unique document_id (avoids collisions in subfolders)
            let document_id = fpath
                .strip_prefix(base_path)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| filename.clone());

            let detected_type = fpath
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();

            if is_unsupported_file(fpath) {
                file_results.push(FileIngestResult {
                    filename: filename.clone(),
                    file_path: file_path.clone(),
                    detected_type: detected_type.clone(),
                    status: FileIngestStatus::SkippedUnsupported,
                    failure_reason: Some(format!(
                        "Format .{detected_type} not supported (Visio requires external tooling)"
                    )),
                    ocr_attempted: false,
                    chunks_created: 0,
                });
                continue;
            }

            let extract = extract_text_from_file(fpath);
            let chunks = chunk_text(&extract.text, CHUNK_SIZE, CHUNK_OVERLAP);
            let num_chunks = chunks.len();

            let status = if !chunks.is_empty() {
                FileIngestStatus::Ingested
            } else if extract.ocr_attempted {
                FileIngestStatus::FailedOcr
            } else if extract.failure_reason.is_some() {
                FileIngestStatus::FailedExtraction
            } else {
                FileIngestStatus::FailedUnknown
            };

            file_results.push(FileIngestResult {
                filename: filename.clone(),
                file_path: file_path.clone(),
                detected_type: extract.file_type.clone(),
                status,
                failure_reason: extract.failure_reason.clone(),
                ocr_attempted: extract.ocr_attempted,
                chunks_created: num_chunks as u64,
            });
            for (chunk_idx, chunk_text) in chunks.into_iter().enumerate() {
                let mut columns = BTreeMap::new();
                columns.insert("document_id".into(), document_id.clone());
                columns.insert("chunk_index".into(), chunk_idx.to_string());
                columns.insert("chunk_text".into(), chunk_text.clone());
                columns.insert("source_file".into(), file_path.clone());
                let page_number = if extract.page_count > 0 && num_chunks > 0 {
                    1 + (chunk_idx as u64 * extract.page_count / num_chunks as u64)
                        .min(extract.page_count.saturating_sub(1))
                } else {
                    0
                };
                columns.insert("page_number".into(), page_number.to_string());
                columns.insert("filename".into(), filename.clone());
                columns.insert("file_path".into(), file_path.clone());
                columns.insert("file_type".into(), extract.file_type.clone());
                columns.insert("file_size_bytes".into(), file_size.to_string());
                columns.insert("content_text".into(), chunk_text);
                columns.insert(
                    "ocr_used".into(),
                    if extract.ocr_used { "1" } else { "0" }.to_string(),
                );

                let entity_id = format!("{}::chunk::{}", document_id, chunk_idx);
                all_chunks.push(IngestRow { entity_id, columns });
            }
        }

        let start = offset as usize;
        let rows: Vec<IngestRow> = all_chunks
            .into_iter()
            .skip(start)
            .take(limit as usize)
            .collect();

        let file_results = if offset == 0 {
            Some(file_results)
        } else {
            None
        };

        Ok(IngestBatchResult {
            table_name: "documents".to_string(),
            rows,
            offset,
            file_results,
        })
    }
}

// ── OneDriveConnector ───────────────────────────────────────────────────────

mod onedrive;

pub use onedrive::{load_onedrive_config, save_onedrive_config, OneDriveConfig, OneDriveConnector};

// ── Column classifier ───────────────────────────────────────────────────────

const PII_PATTERNS: &[&str] = &[
    "email",
    "phone",
    "address",
    "name",
    "dob",
    "date_of_birth",
    "ssn",
    "social_security",
    "iban",
    "sort_code",
    "card_number",
    "gps_latitude",
    "gps_longitude",
    "gps_altitude",
    "file_path",
    "location",
];

const SECRET_PATTERNS: &[&str] = &["api_key", "token", "password", "secret", "credential"];

pub fn classify_column(column_name: &str) -> ColumnClassResult {
    let lower = column_name.to_lowercase();

    for pattern in PII_PATTERNS {
        if lower.contains(pattern) {
            return ColumnClassResult {
                classification: "pii".to_string(),
                is_pii: true,
                is_secret: false,
                suggested_sensitivity: 3,
            };
        }
    }

    for pattern in SECRET_PATTERNS {
        if lower.contains(pattern) {
            return ColumnClassResult {
                classification: "secret".to_string(),
                is_pii: false,
                is_secret: true,
                suggested_sensitivity: 3,
            };
        }
    }

    ColumnClassResult {
        classification: "normal".to_string(),
        is_pii: false,
        is_secret: false,
        suggested_sensitivity: 1,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::NamedTempFile;

    fn create_test_sqlite() -> NamedTempFile {
        let tmp = NamedTempFile::new().unwrap();
        let conn = Connection::open(tmp.path()).unwrap();
        conn.execute_batch(
            "CREATE TABLE users (id INTEGER PRIMARY KEY, name TEXT NOT NULL, email TEXT);
             INSERT INTO users VALUES (1, 'Alice', 'alice@example.com');
             INSERT INTO users VALUES (2, 'Bob', 'bob@example.com');
             INSERT INTO users VALUES (3, 'Charlie', 'charlie@example.com');",
        )
        .unwrap();
        tmp
    }

    #[test]
    fn test_sqlite_inspect() {
        let tmp = create_test_sqlite();
        let c = SQLiteConnector::new("sqlite-test");
        let tables = c.inspect_schema(tmp.path()).unwrap();

        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.table_name, "users");
        assert_eq!(t.row_count_estimate, 3);
        assert_eq!(t.columns.len(), 3);

        assert_eq!(t.columns[0].name, "id");
        assert!(t.columns[0].is_primary_key);
        assert_eq!(t.columns[0].data_type, "INTEGER");

        assert_eq!(t.columns[1].name, "name");
        assert!(!t.columns[1].nullable);

        assert_eq!(t.columns[2].name, "email");
        assert!(t.columns[2].nullable);
    }

    #[test]
    fn test_sqlite_ingest() {
        let tmp = create_test_sqlite();
        let c = SQLiteConnector::new("sqlite-test");

        let batch = c.ingest_batch(tmp.path(), "users", 0, 2).unwrap();
        assert_eq!(batch.table_name, "users");
        assert_eq!(batch.offset, 0);
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].columns["name"], "Alice");
        assert_eq!(batch.rows[1].columns["name"], "Bob");

        let batch2 = c.ingest_batch(tmp.path(), "users", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
        assert_eq!(batch2.rows[0].columns["name"], "Charlie");
    }

    #[test]
    fn test_csv_inspect() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sales.csv"),
            "id,product,amount\n1,Widget,9.99\n2,Gadget,19.99\n",
        )
        .unwrap();

        let c = CsvFolderConnector::new("csv-test");
        let tables = c.inspect_schema(dir.path()).unwrap();

        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.table_name, "sales");
        assert_eq!(t.row_count_estimate, 2);
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.columns[0].name, "id");
        assert_eq!(t.columns[1].name, "product");
        assert_eq!(t.columns[2].name, "amount");
    }

    #[test]
    fn test_csv_ingest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("sales.csv"),
            "id,product,amount\n1,Widget,9.99\n2,Gadget,19.99\n3,Doohickey,4.99\n",
        )
        .unwrap();

        let c = CsvFolderConnector::new("csv-test");
        let batch = c.ingest_batch(dir.path(), "sales", 0, 2).unwrap();
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].columns["product"], "Widget");
        assert_eq!(batch.rows[1].columns["product"], "Gadget");

        let batch2 = c.ingest_batch(dir.path(), "sales", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
        assert_eq!(batch2.rows[0].columns["product"], "Doohickey");
    }

    #[test]
    fn test_json_inspect() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("events.json"),
            "{\"id\":1,\"type\":\"click\",\"ts\":1000}\n{\"id\":2,\"type\":\"view\",\"ts\":2000}\n",
        )
        .unwrap();

        let c = JsonFolderConnector::new("json-test");
        let tables = c.inspect_schema(dir.path()).unwrap();

        assert_eq!(tables.len(), 1);
        let t = &tables[0];
        assert_eq!(t.table_name, "events");
        assert_eq!(t.row_count_estimate, 2);
        assert!(t.columns.len() >= 3);
    }

    #[test]
    fn test_json_ingest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("events.json"),
            "{\"id\":1,\"kind\":\"click\"}\n{\"id\":2,\"kind\":\"view\"}\n{\"id\":3,\"kind\":\"scroll\"}\n",
        )
        .unwrap();

        let c = JsonFolderConnector::new("json-test");
        let batch = c.ingest_batch(dir.path(), "events", 0, 2).unwrap();
        assert_eq!(batch.rows.len(), 2);
        assert_eq!(batch.rows[0].columns["kind"], "click");
        assert_eq!(batch.rows[1].columns["kind"], "view");

        let batch2 = c.ingest_batch(dir.path(), "events", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
        assert_eq!(batch2.rows[0].columns["kind"], "scroll");
    }

    #[test]
    fn test_image_inspect_schema() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("photo1.jpg"), b"fake-jpeg").unwrap();
        fs::write(dir.path().join("photo2.png"), b"fake-png").unwrap();
        fs::write(dir.path().join("notes.txt"), b"not an image").unwrap();

        let c = ImageConnector::new("img-test");
        let tables = c.inspect_schema(dir.path()).unwrap();

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].table_name, "images");
        assert_eq!(tables[0].row_count_estimate, 2);
        assert!(tables[0].columns.len() >= 10);
    }

    #[test]
    fn test_image_ingest_no_exif() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("noexif.jpg"), b"not-a-real-jpeg").unwrap();

        let c = ImageConnector::new("img-test");
        let batch = c.ingest_batch(dir.path(), "images", 0, 10).unwrap();

        assert_eq!(batch.rows.len(), 1);
        assert_eq!(batch.rows[0].columns["filename"], "noexif.jpg");
        assert!(batch.rows[0].columns.contains_key("file_path"));
        assert!(batch.rows[0].columns.contains_key("file_size_bytes"));
    }

    #[test]
    fn test_image_ingest_pagination() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.jpg"), b"img1").unwrap();
        fs::write(dir.path().join("b.jpg"), b"img2").unwrap();
        fs::write(dir.path().join("c.jpg"), b"img3").unwrap();

        let c = ImageConnector::new("img-test");
        let batch1 = c.ingest_batch(dir.path(), "images", 0, 2).unwrap();
        assert_eq!(batch1.rows.len(), 2);

        let batch2 = c.ingest_batch(dir.path(), "images", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
    }

    #[test]
    fn test_document_inspect_schema() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("readme.txt"), b"hello world").unwrap();
        fs::write(dir.path().join("notes.md"), b"# Notes").unwrap();
        fs::write(dir.path().join("legacy.doc"), b"binary").unwrap();
        fs::write(dir.path().join("photo.jpg"), b"not a document").unwrap();

        let c = DocumentConnector::new("doc-test");
        let tables = c.inspect_schema(dir.path()).unwrap();

        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].table_name, "documents");
        assert!(
            tables[0].row_count_estimate >= 6,
            "counts document + unsupported files"
        );
        assert!(tables[0].columns.len() >= 10);
        let col_names: Vec<_> = tables[0].columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"document_id"));
        assert!(col_names.contains(&"chunk_index"));
        assert!(col_names.contains(&"chunk_text"));
    }

    #[test]
    fn test_document_ingest_txt() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.txt"), b"Hello, world!").unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 10).unwrap();

        assert_eq!(batch.rows.len(), 1, "small document remains single chunk");
        assert_eq!(batch.rows[0].columns["filename"], "hello.txt");
        assert_eq!(batch.rows[0].columns["document_id"], "hello.txt");
        assert_eq!(batch.rows[0].columns["chunk_index"], "0");
        assert_eq!(batch.rows[0].columns["file_type"], "txt");
        assert_eq!(batch.rows[0].columns["content_text"], "Hello, world!");
        assert!(
            batch.rows[0].columns["file_size_bytes"]
                .parse::<u64>()
                .unwrap()
                > 0
        );
        let results = batch.file_results.as_ref().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].filename, "hello.txt");
        assert_eq!(results[0].status, FileIngestStatus::Ingested);
        assert_eq!(results[0].chunks_created, 1);
        assert!(!results[0].ocr_attempted);
    }

    #[test]
    fn test_document_ingest_markdown() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("readme.md"),
            b"# Title\n\nSome content here.",
        )
        .unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 10).unwrap();

        assert_eq!(batch.rows.len(), 1);
        assert_eq!(batch.rows[0].columns["file_type"], "md");
        assert!(batch.rows[0].columns["content_text"].contains("Title"));
    }

    #[test]
    fn test_document_ingest_pagination() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"first").unwrap();
        fs::write(dir.path().join("b.txt"), b"second").unwrap();
        fs::write(dir.path().join("c.txt"), b"third").unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch1 = c.ingest_batch(dir.path(), "documents", 0, 2).unwrap();
        assert_eq!(batch1.rows.len(), 2);

        let batch2 = c.ingest_batch(dir.path(), "documents", 2, 10).unwrap();
        assert_eq!(batch2.rows.len(), 1);
    }

    #[test]
    fn test_chunk_text_small_remains_single() {
        let text = "Short text.";
        let chunks = chunk_text(text, CHUNK_SIZE, CHUNK_OVERLAP);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Short text.");
    }

    #[test]
    fn test_chunk_text_large_splits_into_multiple() {
        let text: String = "x".repeat(4000);
        let chunks = chunk_text(&text, CHUNK_SIZE, CHUNK_OVERLAP);
        assert!(
            chunks.len() >= 2,
            "4000 chars should produce multiple chunks, got {}",
            chunks.len()
        );
        for (i, ch) in chunks.iter().enumerate() {
            assert!(
                ch.len() <= CHUNK_SIZE + 100,
                "chunk {} length {} exceeds CHUNK_SIZE",
                i,
                ch.len()
            );
        }
    }

    #[test]
    fn test_chunk_text_overlap() {
        let text: String = "a".repeat(2000);
        let chunks = chunk_text(&text, CHUNK_SIZE, CHUNK_OVERLAP);
        assert!(chunks.len() >= 2);
        let c1_end = chunks[0]
            .chars()
            .rev()
            .take(CHUNK_OVERLAP)
            .collect::<String>();
        let c2_start = chunks[1].chars().take(CHUNK_OVERLAP).collect::<String>();
        assert_eq!(
            c1_end, c2_start,
            "consecutive chunks should overlap by CHUNK_OVERLAP"
        );
    }

    #[test]
    fn test_document_folder_multiple_files_recursive() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"first").unwrap();
        fs::write(dir.path().join("b.txt"), b"second").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("c.txt"), b"third").unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 100).unwrap();

        assert_eq!(batch.rows.len(), 3, "all 3 files from root and subfolder");
        let doc_ids: Vec<_> = batch
            .rows
            .iter()
            .map(|r| r.columns["document_id"].as_str())
            .collect();
        assert!(doc_ids.contains(&"a.txt"));
        assert!(doc_ids.contains(&"b.txt"));
        assert!(doc_ids.contains(&"sub/c.txt"));
        let results = batch.file_results.as_ref().unwrap();
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_document_unsupported_reported() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("diagram.vsdx"), b"fake visio").unwrap();
        fs::write(dir.path().join("readme.txt"), b"supported").unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 100).unwrap();

        assert_eq!(batch.rows.len(), 1, "only txt ingested");
        let results = batch.file_results.as_ref().unwrap();
        assert_eq!(results.len(), 2, "both files reported");
        let visio_result = results
            .iter()
            .find(|r| r.filename == "diagram.vsdx")
            .unwrap();
        assert_eq!(visio_result.status, FileIngestStatus::SkippedUnsupported);
        assert!(visio_result
            .failure_reason
            .as_ref()
            .unwrap()
            .contains("not supported"));
        assert_eq!(visio_result.chunks_created, 0);
        let txt_result = results.iter().find(|r| r.filename == "readme.txt").unwrap();
        assert_eq!(txt_result.status, FileIngestStatus::Ingested);
    }

    #[test]
    fn test_document_empty_file_failed_record() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("empty.txt"), b"").unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 100).unwrap();

        assert_eq!(batch.rows.len(), 0, "empty file produces no chunks");
        let results = batch.file_results.as_ref().unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, FileIngestStatus::FailedExtraction);
        assert!(results[0].failure_reason.is_some());
        assert_eq!(results[0].chunks_created, 0);
    }

    #[test]
    fn test_document_ingest_large_splits_into_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let large_text: String = "word ".repeat(400);
        fs::write(dir.path().join("large.txt"), large_text.as_bytes()).unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 100).unwrap();

        assert!(
            batch.rows.len() >= 2,
            "large document should split into multiple chunks, got {}",
            batch.rows.len()
        );
        assert_eq!(batch.rows[0].columns["document_id"], "large.txt");
        assert_eq!(batch.rows[0].columns["chunk_index"], "0");
        assert_eq!(
            batch.rows[1].columns["document_id"], "large.txt",
            "all chunks from same document"
        );
        assert_eq!(batch.rows[1].columns["chunk_index"], "1");
    }

    #[test]
    fn test_ocr_used_column() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.txt"), b"Hello, world!").unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 10).unwrap();

        assert_eq!(batch.rows.len(), 1);
        assert!(batch.rows[0].columns.contains_key("ocr_used"));
        assert_eq!(
            batch.rows[0].columns["ocr_used"], "0",
            "txt files do not use OCR"
        );
    }

    #[test]
    fn test_scanned_pdf_ocr_path() {
        // Use minimal PDF with short text (< 200 chars) to trigger OCR path.
        // When pdftoppm/tesseract are available, OCR runs and produces text.
        // When OCR tools are missing, we fall back to pdf_oxide text (if any).
        // Skip when fixture missing or when PDF has no extractable text and no OCR tools.
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("minimal.pdf");
        if !fixture_path.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("minimal.pdf");
        fs::copy(&fixture_path, &dest).unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 10).unwrap();

        assert!(!batch.rows.is_empty(), "scanned PDF should produce chunks");
        assert_eq!(batch.rows[0].columns["file_type"], "pdf");
        assert!(batch.rows[0].columns.contains_key("ocr_used"));
        // Chunk text should contain content (from pdf_oxide or OCR)
        assert!(
            !batch.rows[0].columns["chunk_text"].is_empty(),
            "chunk_text should not be empty"
        );
    }

    #[test]
    fn test_document_ingest_with_ocr_metadata() {
        // Verify entity extraction pipeline receives ocr_used in columns.
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("invoice.txt"),
            b"INVOICE #001\nAcme Corp\ncontact@acme.com\nTotal: $500",
        )
        .unwrap();

        let c = DocumentConnector::new("doc-test");
        let batch = c.ingest_batch(dir.path(), "documents", 0, 10).unwrap();

        assert_eq!(batch.rows.len(), 1);
        assert_eq!(batch.rows[0].columns["ocr_used"], "0");
        assert!(batch.rows[0].columns["chunk_text"].contains("Acme"));
    }

    #[test]
    fn test_classify_gps_as_pii() {
        let r = classify_column("gps_latitude");
        assert!(r.is_pii);
        assert_eq!(r.suggested_sensitivity, 3);

        let r = classify_column("gps_longitude");
        assert!(r.is_pii);

        let r = classify_column("file_path");
        assert!(r.is_pii);
    }

    #[test]
    fn test_classify_pii() {
        let r = classify_column("email");
        assert!(r.is_pii);
        assert!(!r.is_secret);
        assert_eq!(r.classification, "pii");
        assert_eq!(r.suggested_sensitivity, 3);

        let r = classify_column("phone_number");
        assert!(r.is_pii);

        let r = classify_column("user_ssn");
        assert!(r.is_pii);

        let r = classify_column("home_address");
        assert!(r.is_pii);

        let r = classify_column("DATE_OF_BIRTH");
        assert!(r.is_pii);
    }

    #[test]
    fn test_classify_secrets() {
        let r = classify_column("api_key");
        assert!(r.is_secret);
        assert!(!r.is_pii);
        assert_eq!(r.classification, "secret");
        assert_eq!(r.suggested_sensitivity, 3);

        let r = classify_column("user_password");
        assert!(r.is_secret);

        let r = classify_column("auth_token");
        assert!(r.is_secret);
    }

    #[test]
    fn test_classify_normal() {
        let r = classify_column("amount");
        assert!(!r.is_pii);
        assert!(!r.is_secret);
        assert_eq!(r.classification, "normal");
        assert_eq!(r.suggested_sensitivity, 1);

        let r = classify_column("status");
        assert!(!r.is_pii);
        assert!(!r.is_secret);
    }
}
