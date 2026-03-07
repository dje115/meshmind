//! OCR fallback for scanned PDFs.
//!
//! When PDF text extraction yields little content (< 200 chars), this module
//! renders pages to images and runs Tesseract OCR to recover text.
//!
//! Requires: poppler-utils (pdftoppm) and tesseract-ocr installed and in PATH.

use std::path::Path;
use std::process::Command;

/// Minimum extractable text length to consider a PDF "scanned" and trigger OCR.
pub const SCANNED_PDF_TEXT_THRESHOLD: usize = 200;

/// Result of OCR on a PDF.
#[derive(Debug, Clone)]
pub struct OcrResult {
    /// Combined text from all pages.
    pub text: String,
    /// Number of pages processed.
    pub page_count: u64,
}

/// Run OCR on a PDF by rendering pages to images and running Tesseract.
///
/// Returns `Some(OcrResult)` on success, `None` if pdftoppm or tesseract
/// are not available, or on error.
pub fn run_pdf_ocr(path: &Path) -> Option<OcrResult> {
    let temp_dir = tempfile::tempdir().ok()?;
    let prefix = temp_dir.path().join("page");

    // Render PDF pages to PNG using pdftoppm (poppler-utils)
    let pdftoppm = which_cmd("pdftoppm")?;
    let output = Command::new(&pdftoppm)
        .args(["-png", "-r", "300"])
        .arg(path)
        .arg(&prefix)
        .output()
        .ok()?;

    if !output.status.success() {
        tracing::debug!(
            "pdftoppm failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }

    // Find generated PNG files (page-1.png, page-2.png, ...)
    let mut page_files: Vec<_> = std::fs::read_dir(temp_dir.path())
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ex| ex == "png").unwrap_or(false))
        .collect();
    page_files.sort_by_key(|e| e.path());

    if page_files.is_empty() {
        return None;
    }

    let tesseract = which_cmd("tesseract")?;

    let mut all_text = String::new();
    for (i, entry) in page_files.iter().enumerate() {
        let img_path = entry.path();
        let page_text = run_tesseract_on_image(&tesseract, &img_path)?;
        if !page_text.is_empty() {
            if !all_text.is_empty() {
                all_text.push_str("\n\n");
            }
            all_text.push_str(&page_text);
            tracing::debug!(page = i + 1, "OCR extracted {} chars", page_text.len());
        }
    }

    let text = if all_text.len() > 100 * 1024 {
        all_text[..100 * 1024].to_string()
    } else {
        all_text
    };

    Some(OcrResult {
        text,
        page_count: page_files.len() as u64,
    })
}

fn run_tesseract_on_image(tesseract: &str, img_path: &Path) -> Option<String> {
    let output = Command::new(tesseract)
        .arg(img_path)
        .arg("stdout")
        .arg("-l")
        .arg("eng")
        .output()
        .ok()?;

    if !output.status.success() {
        tracing::debug!(
            "tesseract failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        return None;
    }

    let text = String::from_utf8(output.stdout).unwrap_or_default();
    Some(text.trim().to_string())
}

/// Find pdftoppm or tesseract in PATH.
fn which_cmd(name: &str) -> Option<String> {
    #[cfg(windows)]
    let which = "where";
    #[cfg(not(windows))]
    let which = "which";

    let output = Command::new(which).arg(name).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8(output.stdout)
        .ok()?
        .lines()
        .next()?
        .trim()
        .to_string();
    if line.is_empty() {
        None
    } else {
        Some(line)
    }
}

/// Check if OCR tools (pdftoppm, tesseract) are available.
#[allow(dead_code)]
pub fn ocr_tools_available() -> bool {
    which_cmd("pdftoppm").is_some() && which_cmd("tesseract").is_some()
}
