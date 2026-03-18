//! Structural and architectural analysis.
//!
//! Detects cyclic dependencies, computes coupling metrics (instability,
//! cohesion, coupling score, API surface ratio), identifies god modules,
//! and flags excessive public API surfaces.

use std::collections::HashMap;

use petgraph::graph::NodeIndex;

use spectron_core::{
    GraphNode, ModuleId, ModuleInfo, ModuleMetrics, Symbol, SymbolId, Visibility,
};
use spectron_graph::{extract_module_subgraph, find_cycles, CallGraphData, GraphSet};

// ---------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------

const GOD_MODULE_SYMBOL_THRESHOLD: u32 = 50;
const GOD_MODULE_COUPLING_THRESHOLD: f32 = 20.0;
const API_SURFACE_THRESHOLD: f32 = 0.8;

// ---------------------------------------------------------------------------
// StructuralFinding — lightweight result before conversion to unified Finding
// ---------------------------------------------------------------------------

/// A structural issue detected during architectural analysis.
#[derive(Clone, Debug)]
pub struct StructuralFinding {
    pub kind: StructuralFindingKind,
    pub severity: Severity,
    pub modules: Vec<ModuleId>,
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StructuralFindingKind {
    CyclicModuleDependency,
    CyclicCrateDependency,
    GodModule,
    ExcessiveApiSurface,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
}

// ---------------------------------------------------------------------------
// StructuralReport
// ---------------------------------------------------------------------------

/// Complete output of the structural analysis pass.
#[derive(Clone, Debug)]
pub struct StructuralReport {
    pub findings: Vec<StructuralFinding>,
    pub cycle_count: usize,
    pub god_module_count: usize,
}

impl StructuralReport {
    pub fn empty() -> Self {
        Self {
            findings: Vec::new(),
            cycle_count: 0,
            god_module_count: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Cycle detection
// ---------------------------------------------------------------------------

/// Detect cyclic dependencies among modules and crates in the structure graph.
///
/// Extracts a module-only subgraph and runs Tarjan SCC. Each SCC with more
/// than one node is reported as a cycle.
pub fn detect_cycles(graph_set: &GraphSet) -> Vec<StructuralFinding> {
    let module_graph = extract_module_subgraph(&graph_set.structure_graph);
    let sccs = find_cycles(&module_graph);

    let node_to_module: HashMap<NodeIndex, ModuleId> = module_graph
        .node_indices()
        .filter_map(|ni| match &module_graph[ni] {
            GraphNode::Module(mid) => Some((ni, *mid)),
            _ => None,
        })
        .collect();

    let mut findings = Vec::new();

    for scc in &sccs {
        let module_ids: Vec<ModuleId> = scc
            .iter()
            .filter_map(|ni| node_to_module.get(ni).copied())
            .collect();

        let is_crate_level = scc.iter().any(|ni| {
            matches!(&module_graph[*ni], GraphNode::Crate(_))
        });

        let severity = if is_crate_level {
            Severity::Critical
        } else if module_ids.len() >= 3 {
            Severity::High
        } else {
            Severity::Medium
        };

        let kind = if is_crate_level {
            StructuralFindingKind::CyclicCrateDependency
        } else {
            StructuralFindingKind::CyclicModuleDependency
        };

        findings.push(StructuralFinding {
            kind,
            severity,
            modules: module_ids,
            description: format!(
                "Cyclic dependency involving {} modules/crates",
                scc.len()
            ),
        });
    }

    findings
}

// ---------------------------------------------------------------------------
// Coupling metrics
// ---------------------------------------------------------------------------

/// Compute architectural coupling metrics for each module and update the
/// metrics in place.
///
/// Populates `instability`, `cohesion`, `coupling_score`, and
/// `api_surface_ratio` on each `ModuleMetrics` entry.
pub fn compute_coupling_metrics(
    module_metrics: &mut HashMap<ModuleId, ModuleMetrics>,
    modules: &HashMap<ModuleId, ModuleInfo>,
    symbols: &HashMap<SymbolId, Symbol>,
    call_graph_data: &CallGraphData,
) {
    let symbol_to_module: HashMap<SymbolId, ModuleId> = symbols
        .iter()
        .map(|(sid, sym)| (*sid, sym.module_id))
        .collect();

    for (mod_id, mm) in module_metrics.iter_mut() {
        let total_fan = mm.fan_in + mm.fan_out;
        let instability = if total_fan > 0 {
            mm.fan_out as f32 / total_fan as f32
        } else {
            0.0
        };

        let coupling_score = total_fan as f32;

        let module_info = modules.get(mod_id);
        let sym_ids: &[SymbolId] = module_info
            .map(|mi| mi.symbol_ids.as_slice())
            .unwrap_or(&[]);

        // Cohesion: ratio of intra-module references to total references.
        let mut internal_refs = 0u32;
        let mut total_refs = 0u32;

        for sid in sym_ids {
            if let Some(callees) = call_graph_data.callees.get(sid) {
                for callee in callees {
                    total_refs += 1;
                    if symbol_to_module.get(callee).copied() == Some(*mod_id) {
                        internal_refs += 1;
                    }
                }
            }
        }

        let cohesion = if total_refs > 0 {
            internal_refs as f32 / total_refs as f32
        } else {
            1.0 // no references means trivially cohesive
        };

        // API surface ratio: public symbols / total symbols.
        let total_symbols = sym_ids.len() as f32;
        let public_symbols = sym_ids
            .iter()
            .filter(|sid| {
                symbols
                    .get(sid)
                    .map(|s| s.visibility == Visibility::Public)
                    .unwrap_or(false)
            })
            .count() as f32;

        let api_surface_ratio = if total_symbols > 0.0 {
            public_symbols / total_symbols
        } else {
            0.0
        };

        mm.instability = Some(instability);
        mm.cohesion = Some(cohesion);
        mm.coupling_score = Some(coupling_score);
        mm.api_surface_ratio = Some(api_surface_ratio);
    }
}

// ---------------------------------------------------------------------------
// God module detection
// ---------------------------------------------------------------------------

/// Detect modules that are both large (high symbol count) and highly coupled.
pub fn detect_god_modules(
    module_metrics: &HashMap<ModuleId, ModuleMetrics>,
) -> Vec<StructuralFinding> {
    let mut findings = Vec::new();

    for (mod_id, mm) in module_metrics {
        let coupling = mm.coupling_score.unwrap_or(0.0);
        if mm.symbol_count > GOD_MODULE_SYMBOL_THRESHOLD
            && coupling > GOD_MODULE_COUPLING_THRESHOLD
        {
            findings.push(StructuralFinding {
                kind: StructuralFindingKind::GodModule,
                severity: Severity::Medium,
                modules: vec![*mod_id],
                description: format!(
                    "Module has {} symbols and coupling score {:.1} — consider splitting",
                    mm.symbol_count, coupling
                ),
            });
        }
    }

    findings
}

// ---------------------------------------------------------------------------
// Excessive API surface detection
// ---------------------------------------------------------------------------

/// Detect modules where more than 80% of symbols are public.
pub fn detect_excessive_api_surface(
    module_metrics: &HashMap<ModuleId, ModuleMetrics>,
    modules: &HashMap<ModuleId, ModuleInfo>,
) -> Vec<StructuralFinding> {
    let mut findings = Vec::new();

    for (mod_id, mm) in module_metrics {
        if mm.symbol_count < 3 {
            continue; // skip trivially small modules
        }

        let ratio = mm.api_surface_ratio.unwrap_or(0.0);
        if ratio > API_SURFACE_THRESHOLD {
            let name = modules
                .get(mod_id)
                .map(|mi| mi.name.as_str())
                .unwrap_or("unknown");
            findings.push(StructuralFinding {
                kind: StructuralFindingKind::ExcessiveApiSurface,
                severity: Severity::Low,
                modules: vec![*mod_id],
                description: format!(
                    "Module '{}' exposes {:.0}% of its symbols publicly",
                    name,
                    ratio * 100.0
                ),
            });
        }
    }

    findings
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Run the full structural analysis pass.
pub fn analyze(
    graph_set: &GraphSet,
    module_metrics: &mut HashMap<ModuleId, ModuleMetrics>,
    modules: &HashMap<ModuleId, ModuleInfo>,
    symbols: &HashMap<SymbolId, Symbol>,
) -> StructuralReport {
    compute_coupling_metrics(module_metrics, modules, symbols, &graph_set.call_graph_data);

    let mut findings = detect_cycles(graph_set);
    let cycle_count = findings.len();

    let god_findings = detect_god_modules(module_metrics);
    let god_module_count = god_findings.len();
    findings.extend(god_findings);

    findings.extend(detect_excessive_api_surface(module_metrics, modules));

    StructuralReport {
        findings,
        cycle_count,
        god_module_count,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use spectron_core::{
        FileId, ModuleId, ModulePath, SourceSpan, Symbol, SymbolAttributes, SymbolKind,
    };
    use spectron_graph::CallGraphData;

    fn make_symbol(id: SymbolId, name: &str, mod_id: ModuleId, vis: Visibility) -> Symbol {
        Symbol {
            id,
            name: name.to_owned(),
            kind: SymbolKind::Function,
            module_id: mod_id,
            file_id: FileId(0),
            span: SourceSpan::new(FileId(0), 1, 0, 10, 0),
            visibility: vis,
            signature: None,
            attributes: SymbolAttributes::empty(),
        }
    }

    #[test]
    fn coupling_metrics_instability() {
        let mod_id = ModuleId(1);
        let sym1 = SymbolId(1);
        let sym2 = SymbolId(2);

        let mut modules = HashMap::new();
        let mut mi = ModuleInfo::new(mod_id, "test_mod", ModulePath::new("crate::test_mod"), None, None);
        mi.symbol_ids = vec![sym1];
        modules.insert(mod_id, mi);

        let other_mod = ModuleId(2);
        let mut mi2 = ModuleInfo::new(other_mod, "other", ModulePath::new("crate::other"), None, None);
        mi2.symbol_ids = vec![sym2];
        modules.insert(other_mod, mi2);

        let mut symbols = HashMap::new();
        symbols.insert(sym1, make_symbol(sym1, "fn1", mod_id, Visibility::Public));
        symbols.insert(sym2, make_symbol(sym2, "fn2", other_mod, Visibility::Private));

        let mut callees = HashMap::new();
        callees.insert(sym1, vec![sym2]); // sym1 calls sym2 (cross-module)
        let callers = HashMap::new();

        let call_graph_data = CallGraphData { callers, callees };

        let mut metrics = HashMap::new();
        metrics.insert(mod_id, ModuleMetrics::new(mod_id, 1, 10, 0, 1));
        metrics.insert(other_mod, ModuleMetrics::new(other_mod, 1, 10, 1, 0));

        compute_coupling_metrics(&mut metrics, &modules, &symbols, &call_graph_data);

        let mm = &metrics[&mod_id];
        assert_eq!(mm.instability, Some(1.0)); // all fan-out, no fan-in
        assert_eq!(mm.coupling_score, Some(1.0)); // 0 + 1
        assert_eq!(mm.api_surface_ratio, Some(1.0)); // 1 public / 1 total
    }

    #[test]
    fn god_module_detected() {
        let mod_id = ModuleId(1);
        let mut metrics = HashMap::new();
        metrics.insert(
            mod_id,
            ModuleMetrics::new(mod_id, 60, 5000, 15, 10)
                .with_architecture_metrics(0.4, 0.5, 25.0, 0.5),
        );

        let findings = detect_god_modules(&metrics);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, StructuralFindingKind::GodModule);
    }

    #[test]
    fn god_module_not_detected_when_small() {
        let mod_id = ModuleId(1);
        let mut metrics = HashMap::new();
        metrics.insert(
            mod_id,
            ModuleMetrics::new(mod_id, 10, 500, 15, 10)
                .with_architecture_metrics(0.4, 0.5, 25.0, 0.5),
        );

        let findings = detect_god_modules(&metrics);
        assert!(findings.is_empty(), "small module should not be flagged");
    }

    #[test]
    fn excessive_api_surface_detected() {
        let mod_id = ModuleId(1);
        let mut modules = HashMap::new();
        let mut mi = ModuleInfo::new(mod_id, "wide_api", ModulePath::new("crate::wide_api"), None, None);
        mi.symbol_ids = vec![SymbolId(1), SymbolId(2), SymbolId(3), SymbolId(4), SymbolId(5)];
        modules.insert(mod_id, mi);

        let mut metrics = HashMap::new();
        metrics.insert(
            mod_id,
            ModuleMetrics::new(mod_id, 5, 100, 0, 0)
                .with_architecture_metrics(0.0, 1.0, 0.0, 0.9),
        );

        let findings = detect_excessive_api_surface(&metrics, &modules);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].kind, StructuralFindingKind::ExcessiveApiSurface);
    }

    #[test]
    fn excessive_api_surface_skips_small_modules() {
        let mod_id = ModuleId(1);
        let mut modules = HashMap::new();
        let mut mi = ModuleInfo::new(mod_id, "tiny", ModulePath::new("crate::tiny"), None, None);
        mi.symbol_ids = vec![SymbolId(1)];
        modules.insert(mod_id, mi);

        let mut metrics = HashMap::new();
        metrics.insert(
            mod_id,
            ModuleMetrics::new(mod_id, 1, 10, 0, 0)
                .with_architecture_metrics(0.0, 1.0, 0.0, 1.0),
        );

        let findings = detect_excessive_api_surface(&metrics, &modules);
        assert!(findings.is_empty(), "tiny module should be skipped");
    }
}
