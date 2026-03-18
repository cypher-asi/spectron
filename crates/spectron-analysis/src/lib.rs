//! spectron-analysis: metrics, security indicators, entrypoint detection,
//! structural analysis.
//!
//! This crate consumes the `GraphSet` from `spectron-graph` and the symbol data
//! from `spectron-parser`, and produces complexity metrics, security reports,
//! entrypoint lists, complexity flags, and structural/architectural findings.

pub mod entrypoints;
pub mod metrics;
pub mod security;
pub mod structural;
pub mod types;

// Re-export the public API at the crate root for convenience.
pub use entrypoints::detect_entrypoints;
pub use metrics::{
    analyze, compute_module_metrics, compute_symbol_metrics, cyclomatic_complexity,
    generate_complexity_flags, line_count, parameter_count,
};
pub use security::detect_security_indicators;
pub use structural::StructuralReport;
pub use types::{AnalysisOutput, ComplexityFlag, ComplexityFlagKind, FlagTarget};
