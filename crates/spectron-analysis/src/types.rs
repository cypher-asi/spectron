//! Analysis output types: AnalysisOutput, ComplexityFlag, FlagTarget, ComplexityFlagKind.

use std::collections::HashMap;

use spectron_core::{ModuleId, ModuleMetrics, SecurityReport, SymbolId, SymbolMetrics};

// ---------------------------------------------------------------------------
// AnalysisOutput
// ---------------------------------------------------------------------------

/// The complete output of the analysis engine.
///
/// Contains per-symbol metrics, per-module metrics, a security report,
/// detected entrypoints, and complexity flags that highlight areas of concern.
pub struct AnalysisOutput {
    /// Per-symbol metrics (complexity, line count, fan-in/out, etc.).
    pub symbol_metrics: HashMap<SymbolId, SymbolMetrics>,
    /// Per-module metrics (symbol count, line count, fan-in/out).
    pub module_metrics: HashMap<ModuleId, ModuleMetrics>,
    /// Security report with all detected indicators.
    pub security_report: SecurityReport,
    /// Symbols identified as entrypoints (e.g. `main`, handlers).
    pub entrypoints: Vec<SymbolId>,
    /// Complexity flags for symbols and modules exceeding thresholds.
    pub complexity_flags: Vec<ComplexityFlag>,
}

// ---------------------------------------------------------------------------
// ComplexityFlag
// ---------------------------------------------------------------------------

/// A flag indicating that a symbol or module exceeds a complexity threshold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComplexityFlag {
    /// The entity that triggered the flag.
    pub target: FlagTarget,
    /// What kind of complexity threshold was exceeded.
    pub kind: ComplexityFlagKind,
    /// The actual measured value.
    pub value: u32,
    /// The threshold that was exceeded.
    pub threshold: u32,
}

// ---------------------------------------------------------------------------
// FlagTarget
// ---------------------------------------------------------------------------

/// Identifies whether a complexity flag applies to a symbol or a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlagTarget {
    /// A specific symbol.
    Symbol(SymbolId),
    /// A specific module.
    Module(ModuleId),
}

// ---------------------------------------------------------------------------
// ComplexityFlagKind
// ---------------------------------------------------------------------------

/// The kind of complexity threshold that was exceeded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComplexityFlagKind {
    /// Cyclomatic complexity exceeds the threshold.
    HighCyclomaticComplexity,
    /// Function line count exceeds the threshold.
    LargeFunction,
    /// Module symbol count exceeds the threshold.
    LargeModule,
    /// Module or symbol fan-in exceeds the threshold.
    HighFanIn,
    /// Module or symbol fan-out exceeds the threshold.
    HighFanOut,
}
