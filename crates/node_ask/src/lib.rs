//! Planner-first ask flow.
//!
//! - AskPlanner: rule-based intent classification and retrieval strategy
//! - AskPlan: structured retrieval plan
//! - EvidenceCollector: executes plan and returns structured evidence

mod ask_plan;
mod ask_planner;
mod evidence_collector;

pub use ask_plan::{AskPlan, RetrievalBudget, RetrievalSource, RetrievalStep};
pub use ask_planner::AskPlanner;
pub use evidence_collector::{collect_evidence, Evidence, EvidenceItem};
