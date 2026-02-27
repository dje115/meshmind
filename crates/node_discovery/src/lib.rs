//! Data source discovery: scan local directories for databases, CSV, JSON.
//!
//! Discovers potential data sources but does NOT ingest by default.
//! Emits DATA_SOURCE_DISCOVERED events for each found source.
