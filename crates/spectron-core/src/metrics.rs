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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Instability index: `fan_out / (fan_in + fan_out)`. Ranges from 0.0
    /// (maximally stable) to 1.0 (maximally unstable).
    pub instability: Option<f32>,
    /// Cohesion score: `internal_references / total_references`. Higher is
    /// more cohesive.
    pub cohesion: Option<f32>,
    /// Coupling score: `fan_in + fan_out`. Higher means more coupled.
    pub coupling_score: Option<f32>,
    /// API surface ratio: `public_symbols / total_symbols`. Values above 0.8
    /// may indicate an overly broad public interface.
    pub api_surface_ratio: Option<f32>,
}

impl ModuleMetrics {
    /// Create a new `ModuleMetrics` with basic counts. Architectural metrics
    /// (`instability`, `cohesion`, `coupling_score`, `api_surface_ratio`) are
    /// initialized to `None` and can be populated later via
    /// [`with_architecture_metrics`](Self::with_architecture_metrics).
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
            instability: None,
            cohesion: None,
            coupling_score: None,
            api_surface_ratio: None,
        }
    }

    /// Populate the architectural metrics derived from fan-in/fan-out and
    /// symbol visibility data.
    pub fn with_architecture_metrics(
        mut self,
        instability: f32,
        cohesion: f32,
        coupling_score: f32,
        api_surface_ratio: f32,
    ) -> Self {
        self.instability = Some(instability);
        self.cohesion = Some(cohesion);
        self.coupling_score = Some(coupling_score);
        self.api_surface_ratio = Some(api_surface_ratio);
        self
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
        assert_eq!(m.instability, None);
        assert_eq!(m.cohesion, None);
        assert_eq!(m.coupling_score, None);
        assert_eq!(m.api_surface_ratio, None);
    }

    #[test]
    fn module_metrics_with_architecture() {
        let m = ModuleMetrics::new(ModuleId(3), 15, 200, 4, 6)
            .with_architecture_metrics(0.6, 0.8, 10.0, 0.5);
        assert_eq!(m.instability, Some(0.6));
        assert_eq!(m.cohesion, Some(0.8));
        assert_eq!(m.coupling_score, Some(10.0));
        assert_eq!(m.api_surface_ratio, Some(0.5));
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
