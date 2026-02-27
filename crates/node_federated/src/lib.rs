//! Federated learning: distributed training without sharing raw data.
//!
//! Modes:
//! - Default: share model deltas only (policy-gated)
//! - Optional: share aggregate stats only
//! - Optional: share curated anonymized synthetic examples
