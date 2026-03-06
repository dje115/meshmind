//! Integration test: Ingest document fixtures, verify search returns correct results
//! for questions like "how many invoices do I have?"
//!
//! Validates: DocumentConnector -> IngestJob -> ArtifactPublished -> artifacts_fts -> search

use node_connectors::{Connector, DocumentConnector};
use node_ingest::{run_ingest, IngestConfig, IngestJob};
use node_storage::cas::CasStore;
use node_storage::event_log::EventLog;
use node_storage::search;
use node_storage::sqlite_views;

#[test]
fn document_ingest_and_search_invoices() {
    let tmp = tempfile::TempDir::new().unwrap();
    let doc_dir = tmp.path().join("documents");
    std::fs::create_dir_all(&doc_dir).unwrap();

    // Create invoice fixtures (same content as seed/public/documents)
    for i in 1..=3 {
        let content = format!(
            "INVOICE #{:03}\nDate: 2024-01-15\nVendor: Acme Supplies\nClient: Widget Corp\nTotal: $187.00",
            i
        );
        std::fs::write(doc_dir.join(format!("invoice_{:03}.txt", i)), content).unwrap();
    }
    // Add report.txt for document count
    std::fs::write(
        doc_dir.join("report.txt"),
        "Q1 2024 Business Report. Revenue increased 12%.",
    )
    .unwrap();

    let event_log_path = tmp.path().join("events");
    std::fs::create_dir_all(&event_log_path).unwrap();
    let mut event_log = EventLog::open(&event_log_path).unwrap();
    let cas = CasStore::open(tmp.path()).unwrap();
    let db_path = tmp.path().join("sqlite").join("meshmind.db");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let _ = sqlite_views::open_db(&db_path).unwrap();

    let connector = DocumentConnector::new("document");
    let tables = connector.inspect_schema(&doc_dir).unwrap();
    let table_names: Vec<String> = tables.iter().map(|t| t.table_name.clone()).collect();

    let job = IngestJob {
        ingest_id: "test-ingest-001".into(),
        source_id: "test-source".into(),
        connector_type: "document".into(),
    };

    let config = IngestConfig::default();
    let result = run_ingest(
        &job,
        &connector,
        &doc_dir,
        &table_names,
        &config,
        &cas,
        &mut event_log,
        &db_path,
        "test-node",
        None,
    )
    .unwrap();

    assert!(result.success);
    assert_eq!(
        result.documents_created, 4,
        "expected 4 documents (3 invoices + report)"
    );

    // Search for "invoice" - should return 3 hits
    let search_conn = rusqlite::Connection::open(&db_path).unwrap();
    let hits = search::search_artifacts(&search_conn, "invoice", 20).unwrap();
    assert!(
        hits.len() >= 3,
        "expected at least 3 artifact hits for 'invoice', got {}",
        hits.len()
    );

    // Search for "report" - should return 1 hit
    let report_hits = search::search_artifacts(&search_conn, "report", 20).unwrap();
    assert!(
        !report_hits.is_empty(),
        "expected at least 1 hit for 'report'"
    );
}
