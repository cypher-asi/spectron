//! Metric types for symbols and modules.
//!
//! These types capture quantitative measurements computed during analysis,
//! such as cyclomatic complexity, line counts, and coupling metrics.

use serde::{Deserialize, Serialize};

use crate::id::{ModuleId, SymbolId};

// ---------------------------------------------------------------------------
// SymbolMetrics
// ---------------------------------------------------------------------------

/// Quantitative metrics for a single symbol (typically a function or method).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolMetrics {
    /// The symbol these metrics apply to.
    pub symbol_id: SymbolId,
    /// Cyclomatic complexity (number of linearly independent paths).
    pub cyclomatic_complexity: u32,
    /// Total number of source lines occupied by the symbol.
    pub line_count: u32,
    /// Number of parameters (for functions/methods; 0 for other symbol kinds).
    pub parameter_count: u32,
    /// Fan-in: number of distinct callers of this symbol.
    pub fan_in: u32,
    /// Fan-out: number of distinct callees of this symbol.
    pub fan_out: u32,
}

impl SymbolMetrics {
    /// Create a new `SymbolMetrics`.
    pub fn new(
        symbol_id: SymbolId,
        cyclomatic_complexity: u32,
        line_count: u32,
        parameter_count: u32,
    ) -> Self {
        Self {
            symbol_id,
            cyclomatic_complexity,
            line_count,
            parameter_count,
            fan_in: 0,
            fan_out: 0,
        }
    }

    /// Create a new `SymbolMetrics` with fan-in and fan-out values.
    pub fn with_fan(
        symbol_id: SymbolId,
        cyclomatic_complexity: u32,
        line_count: u32,
        parameter_count: u32,
        fan_in: u32,
        fan_out: u32,
    ) -> Self {
        Self {
            symbol_id,
            cyclomatic_complexity,
            line_count,
            parameter_count,
            fan_in,
            fan_out,
        }
    }
}

// ---------------------------------------------------------------------------
// ModuleMetrics
// ---------------------------------------------------------------------------

/// Quantitative metrics for a single module.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleMetrics {
    /// The module these metrics apply to.
    pub module_id: ModuleId,
    /// Total number of symbols declared in the module.
    pub symbol_count: u32,
    /// Total number of source lines in the module.
    pub line_count: u32,
    /// Fan-in: number of other modules that depend on this module.
    pub fan_in: u32,
    /// Fan-out: number of other modules this module depends on.
    pub fan_out: u32,
}

impl ModuleMetrics {
    /// Create a new `ModuleMetrics`.
    pub fn new(
        module_id: ModuleId,
        symbol_count: u32,
        line_count: u32,
        fan_in: u32,
        fan_out: u32,
    ) -> Self {
        Self {
            module_id,
            symbol_count,
            line_count,
            fan_in,
            fan_out,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbol_metrics_construction() {
        let m = SymbolMetrics::new(SymbolId(1), 5, 42, 3);
        assert_eq!(m.symbol_id, SymbolId(1));
        assert_eq!(m.cyclomatic_complexity, 5);
        assert_eq!(m.line_count, 42);
        assert_eq!(m.parameter_count, 3);
    }

    #[test]
    fn symbol_metrics_equality() {
        let a = SymbolMetrics::new(SymbolId(1), 5, 42, 3);
        let b = SymbolMetrics::new(SymbolId(1), 5, 42, 3);
        let c = SymbolMetrics::new(SymbolId(1), 10, 42, 3);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn symbol_metrics_clone() {
        let m = SymbolMetrics::new(SymbolId(7), 2, 10, 1);
        let cloned = m.clone();
        assert_eq!(m, cloned);
    }

    #[test]
    fn module_metrics_construction() {
        let m = ModuleMetrics::new(ModuleId(3), 15, 200, 4, 6);
        assert_eq!(m.module_id, ModuleId(3));
        assert_eq!(m.symbol_count, 15);
        assert_eq!(m.line_count, 200);
        assert_eq!(m.fan_in, 4);
        assert_eq!(m.fan_out, 6);
    }

    #[test]
    fn module_metrics_equality() {
        let a = ModuleMetrics::new(ModuleId(1), 10, 100, 2, 3);
        let b = ModuleMetrics::new(ModuleId(1), 10, 100, 2, 3);
        let c = ModuleMetrics::new(ModuleId(1), 10, 100, 2, 4);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn module_metrics_clone() {
        let m = ModuleMetrics::new(ModuleId(5), 8, 120, 1, 2);
        let cloned = m.clone();
        assert_eq!(m, cloned);
    }

    #[test]
    fn serde_roundtrip_symbol_metrics() {
        let m = SymbolMetrics::new(SymbolId(42), 8, 100, 5);
        let json = serde_json::to_string(&m).expect("serialize failed");
        let deser: SymbolMetrics = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(m, deser);
    }

    #[test]
    fn serde_roundtrip_module_metrics() {
        let m = ModuleMetrics::new(ModuleId(99), 20, 500, 7, 3);
        let json = serde_json::to_string(&m).expect("serialize failed");
        let deser: ModuleMetrics = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(m, deser);
    }
}
