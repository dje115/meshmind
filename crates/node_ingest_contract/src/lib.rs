//! Normalized ingestion contract between ingestion agents and MeshMind core.
//!
//! Language-neutral, transport-neutral; implemented over localhost HTTP/JSON.
//! Version: 1

pub mod types;

pub use types::{
    IngestItemStatus, IngestJob, IngestJobCounts, IngestedChunk, IngestedItem, SourceWatch,
};
