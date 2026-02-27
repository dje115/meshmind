//! Ingestion pipelines: convert raw data into normalized Documents and Facts.
//!
//! Runs incremental, resumable ingestion jobs per approved SourceProfile.
//! Stores content in CAS, emits events, projects into SQLite views.
