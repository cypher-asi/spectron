//! Complexity metrics computation.
//!
//! This module implements the core metric calculations:
//! - Cyclomatic complexity from control flow graphs
//! - Line count from source spans
//! - Parameter count from function signatures
//! - Module size (sum of symbol line counts)
//! - Fan-in / fan-out from call graph data

use std::collections::{HashMap, HashSet};

use petgraph::algo::connected_components;

use spectron_core::{
    ModuleId, ModuleInfo, ModuleMetrics, Symbol, SymbolId, SymbolKind, SymbolMetrics,
};
use spectron_graph::{CallGraphData, ControlFlowGraph, GraphSet};

use crate::types::{
    AnalysisOutput, ComplexityFlag, ComplexityFlagKind, FlagTarget,
};

// ---------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------

/// Cyclomatic complexity threshold. Functions exceeding this are flagged.
const CYCLOMATIC_COMPLEXITY_THRESHOLD: u32 = 15;

/// Function line count threshold. Functions exceeding this are flagged.
const LARGE_FUNCTION_THRESHOLD: u32 = 100;

/// Module symbol count threshold. Modules exceeding this are flagged.
const LARGE_MODULE_THRESHOLD: u32 = 50;

/// Module fan-in threshold. Modules exceeding this are flagged.
const MODULE_FAN_IN_THRESHOLD: u32 = 20;

/// Module fan-out threshold. Modules exceeding this are flagged.
const MODULE_FAN_OUT_THRESHOLD: u32 = 15;

// ---------------------------------------------------------------------------
// Cyclomatic Complexity
// ---------------------------------------------------------------------------

/// Compute cyclomatic complexity from a control flow graph.
///
/// Formula: M = E - N + 2P
/// Where:
/// - E = number of edges
/// - N = number of nodes
/// - P = number of connected components (typically 1)
///
/// If the CFG is empty or has no nodes, returns 0.
pub fn cyclomatic_complexity(cfg: &ControlFlowGraph) -> u32 {
    let graph = &cfg.graph;
    let n = graph.node_count();
    let e = graph.edge_count();

    if n == 0 {
        return 0;
    }

    let p = connected_components(graph) as usize;

    // The formula can underflow if the graph is degenerate (e.g., single node, no edges).
    // In that case, complexity is 1 (one path through the function).
    let result = (e as i64) - (n as i64) + (2 * p as i64);
    if result < 1 {
        1
    } else {
        result as u32
    }
}

// ---------------------------------------------------------------------------
// Line Count
// ---------------------------------------------------------------------------

/// Compute the line count for a symbol from its source span.
///
/// Returns `end_line - start_line + 1`. Both lines are 1-based and inclusive.
pub fn line_count(symbol: &Symbol) -> u32 {
    let span = &symbol.span;
    if span.end_line >= span.start_line {
        span.end_line - span.start_line + 1
    } else {
        // Defensive: if the span is malformed, return 1.
        1
    }
}

// ---------------------------------------------------------------------------
// Parameter Count
// ---------------------------------------------------------------------------

/// Count the number of parameters in a function signature string.
///
/// Parses the signature by extracting the content between the first pair of
/// parentheses and counting comma-separated segments, handling nested
/// generics (`<>`) and nested parentheses.
///
/// Edge cases:
/// - No signature -> 0
/// - Empty parens `fn foo()` -> 0
/// - `fn foo(x: i32)` -> 1
/// - `fn foo(x: i32, y: String)` -> 2
/// - `fn foo(x: HashMap<K, V>, y: i32)` -> 2 (commas inside `<>` are ignored)
/// - `fn foo(&self)` -> 1 (self counts as a parameter)
/// - `fn foo(&self, x: i32)` -> 2
pub fn parameter_count(signature: Option<&str>) -> u32 {
    let sig = match signature {
        Some(s) => s,
        None => return 0,
    };

    // Find the first '(' and matching ')'.
    let open = match sig.find('(') {
        Some(pos) => pos,
        None => return 0,
    };

    // Find the matching close paren, respecting nesting.
    let after_open = &sig[open + 1..];
    let mut depth = 1i32;
    let mut close_offset = None;
    for (i, ch) in after_open.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    close_offset = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }

    let params_str = match close_offset {
        Some(offset) => &after_open[..offset],
        None => after_open, // Malformed, best effort
    };

    let trimmed = params_str.trim();
    if trimmed.is_empty() {
        return 0;
    }

    // Count commas at the top level (not inside angle brackets or parentheses).
    let mut count = 1u32;
    let mut angle_depth = 0i32;
    let mut paren_depth = 0i32;

    for ch in trimmed.chars() {
        match ch {
            '<' => angle_depth += 1,
            '>' => {
                if angle_depth > 0 {
                    angle_depth -= 1;
                }
            }
            '(' => paren_depth += 1,
            ')' => {
                if paren_depth > 0 {
                    paren_depth -= 1;
                }
            }
            ',' if angle_depth == 0 && paren_depth == 0 => {
                count += 1;
            }
            _ => {}
        }
    }

    count
}

// ---------------------------------------------------------------------------
// Symbol Metrics
// ---------------------------------------------------------------------------

/// Compute `SymbolMetrics` for all symbols.
///
/// For each symbol, computes:
/// - Cyclomatic complexity (from CFG if available, 0 otherwise)
/// - Line count (from source span)
/// - Parameter count (from signature)
/// - Fan-in (number of distinct callers from call graph data)
/// - Fan-out (number of distinct callees from call graph data)
pub fn compute_symbol_metrics(
    symbols: &HashMap<SymbolId, Symbol>,
    cfgs: &HashMap<SymbolId, ControlFlowGraph>,
    call_graph_data: &CallGraphData,
) -> HashMap<SymbolId, SymbolMetrics> {
    let mut metrics = HashMap::new();

    for (sym_id, symbol) in symbols {
        // Cyclomatic complexity: only for functions/methods with a CFG.
        let cc = match (symbol.kind == SymbolKind::Function || symbol.kind == SymbolKind::Method, cfgs.get(sym_id)) {
            (true, Some(cfg)) => cyclomatic_complexity(cfg),
            (true, None) => {
                tracing::warn!(
                    symbol_id = sym_id.0,
                    name = %symbol.name,
                    "no CFG available for function/method; setting cyclomatic complexity to 0"
                );
                0
            }
            _ => 0,
        };

        let lc = line_count(symbol);
        let pc = parameter_count(symbol.signature.as_deref());

        // Fan-in: number of distinct callers
        let fan_in = call_graph_data
            .callers
            .get(sym_id)
            .map(|callers| {
                let distinct: HashSet<&SymbolId> = callers.iter().collect();
                distinct.len() as u32
            })
            .unwrap_or(0);

        // Fan-out: number of distinct callees
        let fan_out = call_graph_data
            .callees
            .get(sym_id)
            .map(|callees| {
                let distinct: HashSet<&SymbolId> = callees.iter().collect();
                distinct.len() as u32
            })
            .unwrap_or(0);

        metrics.insert(
            *sym_id,
            SymbolMetrics::with_fan(*sym_id, cc, lc, pc, fan_in, fan_out),
        );
    }

    metrics
}

// ---------------------------------------------------------------------------
// Module Metrics
// ---------------------------------------------------------------------------

/// Compute `ModuleMetrics` for all modules.
///
/// For each module:
/// - symbol_count: number of symbols declared in the module
/// - line_count: sum of line counts of all contained symbols
/// - fan_in: number of distinct external modules that call into this module
/// - fan_out: number of distinct external modules that this module calls
pub fn compute_module_metrics(
    modules: &HashMap<ModuleId, ModuleInfo>,
    symbols: &HashMap<SymbolId, Symbol>,
    symbol_metrics: &HashMap<SymbolId, SymbolMetrics>,
    call_graph_data: &CallGraphData,
) -> HashMap<ModuleId, ModuleMetrics> {
    // Build a lookup from SymbolId to ModuleId for module fan-in/fan-out.
    let symbol_to_module: HashMap<SymbolId, ModuleId> = symbols
        .iter()
        .map(|(sid, sym)| (*sid, sym.module_id))
        .collect();

    let mut metrics = HashMap::new();

    for (mod_id, module) in modules {
        let symbol_count = module.symbol_ids.len() as u32;

        let total_line_count: u32 = module
            .symbol_ids
            .iter()
            .filter_map(|sid| symbol_metrics.get(sid))
            .map(|m| m.line_count)
            .sum();

        // Module fan-in: count distinct external modules that call into this module.
        // For each symbol in this module, look at its callers. If a caller belongs
        // to a different module, that module contributes to fan-in.
        let mut fan_in_modules: HashSet<ModuleId> = HashSet::new();
        for sym_id in &module.symbol_ids {
            if let Some(callers) = call_graph_data.callers.get(sym_id) {
                for caller_id in callers {
                    if let Some(&caller_mod) = symbol_to_module.get(caller_id) {
                        if caller_mod != *mod_id {
                            fan_in_modules.insert(caller_mod);
                        }
                    }
                }
            }
        }

        // Module fan-out: count distinct external modules that this module calls.
        // For each symbol in this module, look at its callees. If a callee belongs
        // to a different module, that module contributes to fan-out.
        let mut fan_out_modules: HashSet<ModuleId> = HashSet::new();
        for sym_id in &module.symbol_ids {
            if let Some(callees) = call_graph_data.callees.get(sym_id) {
                for callee_id in callees {
                    if let Some(&callee_mod) = symbol_to_module.get(callee_id) {
                        if callee_mod != *mod_id {
                            fan_out_modules.insert(callee_mod);
                        }
                    }
                }
            }
        }

        metrics.insert(
            *mod_id,
            ModuleMetrics::new(
                *mod_id,
                symbol_count,
                total_line_count,
                fan_in_modules.len() as u32,
                fan_out_modules.len() as u32,
            ),
        );
    }

    metrics
}

// ---------------------------------------------------------------------------
// Complexity Flags
// ---------------------------------------------------------------------------

/// Generate complexity flags for symbols and modules that exceed thresholds.
pub fn generate_complexity_flags(
    symbol_metrics: &HashMap<SymbolId, SymbolMetrics>,
    module_metrics: &HashMap<ModuleId, ModuleMetrics>,
) -> Vec<ComplexityFlag> {
    let mut flags = Vec::new();

    for (sym_id, sm) in symbol_metrics {
        if sm.cyclomatic_complexity > CYCLOMATIC_COMPLEXITY_THRESHOLD {
            flags.push(ComplexityFlag {
                target: FlagTarget::Symbol(*sym_id),
                kind: ComplexityFlagKind::HighCyclomaticComplexity,
                value: sm.cyclomatic_complexity,
                threshold: CYCLOMATIC_COMPLEXITY_THRESHOLD,
            });
        }

        if sm.line_count > LARGE_FUNCTION_THRESHOLD {
            flags.push(ComplexityFlag {
                target: FlagTarget::Symbol(*sym_id),
                kind: ComplexityFlagKind::LargeFunction,
                value: sm.line_count,
                threshold: LARGE_FUNCTION_THRESHOLD,
            });
        }
    }

    for (mod_id, mm) in module_metrics {
        if mm.symbol_count > LARGE_MODULE_THRESHOLD {
            flags.push(ComplexityFlag {
                target: FlagTarget::Module(*mod_id),
                kind: ComplexityFlagKind::LargeModule,
                value: mm.symbol_count,
                threshold: LARGE_MODULE_THRESHOLD,
            });
        }

        if mm.fan_in > MODULE_FAN_IN_THRESHOLD {
            flags.push(ComplexityFlag {
                target: FlagTarget::Module(*mod_id),
                kind: ComplexityFlagKind::HighFanIn,
                value: mm.fan_in,
                threshold: MODULE_FAN_IN_THRESHOLD,
            });
        }

        if mm.fan_out > MODULE_FAN_OUT_THRESHOLD {
            flags.push(ComplexityFlag {
                target: FlagTarget::Module(*mod_id),
                kind: ComplexityFlagKind::HighFanOut,
                value: mm.fan_out,
                threshold: MODULE_FAN_OUT_THRESHOLD,
            });
        }
    }

    flags
}

// ---------------------------------------------------------------------------
// Top-level analysis
// ---------------------------------------------------------------------------

/// Run all analysis passes on the graph set.
///
/// This is the main entry point for analysis. It computes symbol metrics,
/// module metrics (including coupling), generates complexity flags, detects
/// entrypoints, runs security indicator detection, and performs structural
/// analysis (cycles, god modules, API surface).
pub fn analyze(
    graph_set: &GraphSet,
    symbols: &HashMap<SymbolId, Symbol>,
    modules: &HashMap<ModuleId, ModuleInfo>,
) -> AnalysisOutput {
    let symbol_metrics = compute_symbol_metrics(
        symbols,
        &graph_set.control_flow_graphs,
        &graph_set.call_graph_data,
    );

    let mut module_metrics = compute_module_metrics(
        modules,
        symbols,
        &symbol_metrics,
        &graph_set.call_graph_data,
    );

    let complexity_flags = generate_complexity_flags(&symbol_metrics, &module_metrics);

    let entrypoints = crate::entrypoints::detect_entrypoints(
        symbols,
        modules,
        &graph_set.call_graph_data,
    );

    let security_report = crate::security::detect_security_indicators(
        symbols,
        modules,
        &graph_set.call_graph_data,
    );

    let structural_report = crate::structural::analyze(
        graph_set,
        &mut module_metrics,
        modules,
        symbols,
    );

    AnalysisOutput {
        symbol_metrics,
        module_metrics,
        security_report,
        entrypoints,
        complexity_flags,
        structural_report,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use petgraph::graph::DiGraph;

    use spectron_core::{
        FileId, ModuleId, ModuleInfo, ModulePath, SourceSpan, Symbol,
        SymbolAttributes, SymbolId, SymbolKind, Visibility,
    };
    use spectron_graph::{CfgEdge, CfgNode, ControlFlowGraph};

    // -----------------------------------------------------------------------
    // Helper: build a CFG from a node/edge description
    // -----------------------------------------------------------------------

    fn make_cfg(
        function_id: SymbolId,
        node_count: usize,
        edges: &[(usize, usize)],
    ) -> ControlFlowGraph {
        let mut graph = DiGraph::new();
        let nodes: Vec<_> = (0..node_count)
            .map(|i| {
                if i == 0 {
                    graph.add_node(CfgNode::Entry)
                } else if i == node_count - 1 {
                    graph.add_node(CfgNode::Exit)
                } else {
                    graph.add_node(CfgNode::Statement {
                        span: SourceSpan::new(FileId(0), 1, 0, 1, 0),
                    })
                }
            })
            .collect();

        for &(from, to) in edges {
            graph.add_edge(nodes[from], nodes[to], CfgEdge::Sequential);
        }

        ControlFlowGraph {
            function_id,
            graph,
        }
    }

    // -----------------------------------------------------------------------
    // Test: Linear CFG (5 nodes, 4 edges) -> cyclomatic complexity = 1
    // -----------------------------------------------------------------------

    #[test]
    fn cyclomatic_complexity_linear_cfg() {
        // Linear: Entry -> S1 -> S2 -> S3 -> Exit
        // 5 nodes, 4 edges, 1 connected component
        // M = 4 - 5 + 2*1 = 1
        let cfg = make_cfg(
            SymbolId(1),
            5,
            &[(0, 1), (1, 2), (2, 3), (3, 4)],
        );
        let cc = cyclomatic_complexity(&cfg);
        assert_eq!(cc, 1, "linear CFG with 5 nodes, 4 edges should have complexity 1");
    }

    // -----------------------------------------------------------------------
    // Test: CFG with one branch (6 nodes, 7 edges) -> complexity = 3
    // -----------------------------------------------------------------------

    #[test]
    fn cyclomatic_complexity_branching_cfg() {
        // Entry -> Branch -> TrueBody -> Join -> Exit
        //                 -> FalseBody ->
        // 6 nodes, 7 edges, 1 connected component
        // M = 7 - 6 + 2*1 = 3
        let cfg = make_cfg(
            SymbolId(2),
            6,
            &[
                (0, 1), // Entry -> Branch
                (1, 2), // Branch -> TrueBody
                (1, 3), // Branch -> FalseBody
                (2, 4), // TrueBody -> Join
                (3, 4), // FalseBody -> Join
                (4, 5), // Join -> Exit
                // Extra edge to make 7 total: a back edge or second path
                (1, 4), // Branch -> Join (direct fallthrough)
            ],
        );
        let cc = cyclomatic_complexity(&cfg);
        assert_eq!(cc, 3, "CFG with 6 nodes, 7 edges should have complexity 3");
    }

    // -----------------------------------------------------------------------
    // Test: Empty CFG -> complexity 0
    // -----------------------------------------------------------------------

    #[test]
    fn cyclomatic_complexity_empty_cfg() {
        let cfg = ControlFlowGraph {
            function_id: SymbolId(3),
            graph: DiGraph::new(),
        };
        let cc = cyclomatic_complexity(&cfg);
        assert_eq!(cc, 0, "empty CFG should have complexity 0");
    }

    // -----------------------------------------------------------------------
    // Test: Line count
    // -----------------------------------------------------------------------

    #[test]
    fn line_count_normal() {
        let sym = make_test_symbol(SymbolId(1), "foo", 10, 25, None);
        assert_eq!(line_count(&sym), 16, "lines 10..=25 = 16 lines");
    }

    #[test]
    fn line_count_single_line() {
        let sym = make_test_symbol(SymbolId(1), "foo", 5, 5, None);
        assert_eq!(line_count(&sym), 1, "single line span = 1 line");
    }

    // -----------------------------------------------------------------------
    // Test: Parameter count
    // -----------------------------------------------------------------------

    #[test]
    fn param_count_no_signature() {
        assert_eq!(parameter_count(None), 0);
    }

    #[test]
    fn param_count_empty_parens() {
        assert_eq!(parameter_count(Some("fn foo()")), 0);
    }

    #[test]
    fn param_count_one_param() {
        assert_eq!(parameter_count(Some("fn foo(x: i32)")), 1);
    }

    #[test]
    fn param_count_two_params() {
        assert_eq!(parameter_count(Some("fn foo(x: i32, y: String)")), 2);
    }

    #[test]
    fn param_count_generics_with_commas() {
        assert_eq!(
            parameter_count(Some("fn foo(x: HashMap<K, V>, y: i32)")),
            2,
            "commas inside angle brackets should not be counted"
        );
    }

    #[test]
    fn param_count_self() {
        assert_eq!(parameter_count(Some("fn run(&self)")), 1);
    }

    #[test]
    fn param_count_self_plus_params() {
        assert_eq!(parameter_count(Some("fn run(&self, x: i32, y: bool)")), 3);
    }

    #[test]
    fn param_count_nested_generics() {
        assert_eq!(
            parameter_count(Some("fn foo(x: Vec<HashMap<K, V>>, y: i32)")),
            2,
            "deeply nested generics should be handled"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Fan-in / fan-out from call graph data
    // -----------------------------------------------------------------------

    #[test]
    fn fan_in_fan_out_from_call_graph() {
        // Set up: A calls B, C calls B, B calls D
        // B fan-in = 2 (A and C), B fan-out = 1 (D)
        // A fan-in = 0, A fan-out = 1 (B)
        // C fan-in = 0, C fan-out = 1 (B)
        // D fan-in = 1 (B), D fan-out = 0

        let sym_a = SymbolId(1);
        let sym_b = SymbolId(2);
        let sym_c = SymbolId(3);
        let sym_d = SymbolId(4);

        let mut callers: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        callers.insert(sym_a, vec![]);
        callers.insert(sym_b, vec![sym_a, sym_c]);
        callers.insert(sym_c, vec![]);
        callers.insert(sym_d, vec![sym_b]);

        let mut callees: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        callees.insert(sym_a, vec![sym_b]);
        callees.insert(sym_b, vec![sym_d]);
        callees.insert(sym_c, vec![sym_b]);
        callees.insert(sym_d, vec![]);

        let call_graph_data = CallGraphData { callers, callees };

        let mut symbols = HashMap::new();
        for (id, name) in [(sym_a, "a"), (sym_b, "b"), (sym_c, "c"), (sym_d, "d")] {
            symbols.insert(id, make_test_symbol(id, name, 1, 10, Some("fn foo()")));
        }

        let cfgs = HashMap::new();
        let metrics = compute_symbol_metrics(&symbols, &cfgs, &call_graph_data);

        assert_eq!(metrics[&sym_a].fan_in, 0);
        assert_eq!(metrics[&sym_a].fan_out, 1);
        assert_eq!(metrics[&sym_b].fan_in, 2);
        assert_eq!(metrics[&sym_b].fan_out, 1);
        assert_eq!(metrics[&sym_c].fan_in, 0);
        assert_eq!(metrics[&sym_c].fan_out, 1);
        assert_eq!(metrics[&sym_d].fan_in, 1);
        assert_eq!(metrics[&sym_d].fan_out, 0);
    }

    // -----------------------------------------------------------------------
    // Test: Module metrics computation
    // -----------------------------------------------------------------------

    #[test]
    fn module_metrics_basic() {
        let mod_a = ModuleId(10);
        let mod_b = ModuleId(20);

        let sym1 = SymbolId(1); // in mod_a
        let sym2 = SymbolId(2); // in mod_a
        let sym3 = SymbolId(3); // in mod_b

        let mut modules = HashMap::new();
        let mut mod_a_info = ModuleInfo::new(
            mod_a,
            "mod_a",
            ModulePath::new("crate::mod_a"),
            None,
            None,
        );
        mod_a_info.symbol_ids = vec![sym1, sym2];
        modules.insert(mod_a, mod_a_info);

        let mut mod_b_info = ModuleInfo::new(
            mod_b,
            "mod_b",
            ModulePath::new("crate::mod_b"),
            None,
            None,
        );
        mod_b_info.symbol_ids = vec![sym3];
        modules.insert(mod_b, mod_b_info);

        let mut symbols = HashMap::new();
        symbols.insert(sym1, make_test_symbol(sym1, "fn1", 1, 20, None));  // 20 lines
        symbols.insert(sym2, make_test_symbol(sym2, "fn2", 21, 30, None)); // 10 lines
        symbols.insert(sym3, make_test_symbol(sym3, "fn3", 1, 50, None));  // 50 lines
        // Override module_ids
        symbols.get_mut(&sym1).unwrap().module_id = mod_a;
        symbols.get_mut(&sym2).unwrap().module_id = mod_a;
        symbols.get_mut(&sym3).unwrap().module_id = mod_b;

        let mut sym_metrics = HashMap::new();
        sym_metrics.insert(sym1, SymbolMetrics::new(sym1, 0, 20, 0));
        sym_metrics.insert(sym2, SymbolMetrics::new(sym2, 0, 10, 0));
        sym_metrics.insert(sym3, SymbolMetrics::new(sym3, 0, 50, 0));

        // sym1 (mod_a) calls sym3 (mod_b)
        let mut callers: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        callers.insert(sym1, vec![]);
        callers.insert(sym2, vec![]);
        callers.insert(sym3, vec![sym1]);

        let mut callees: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
        callees.insert(sym1, vec![sym3]);
        callees.insert(sym2, vec![]);
        callees.insert(sym3, vec![]);

        let call_graph_data = CallGraphData { callers, callees };

        let mod_metrics = compute_module_metrics(
            &modules,
            &symbols,
            &sym_metrics,
            &call_graph_data,
        );

        // mod_a: 2 symbols, 30 lines, fan_out=1 (calls mod_b), fan_in=0
        let ma = &mod_metrics[&mod_a];
        assert_eq!(ma.symbol_count, 2);
        assert_eq!(ma.line_count, 30);
        assert_eq!(ma.fan_out, 1, "mod_a calls into mod_b");
        assert_eq!(ma.fan_in, 0, "no external module calls into mod_a");

        // mod_b: 1 symbol, 50 lines, fan_in=1 (called by mod_a), fan_out=0
        let mb = &mod_metrics[&mod_b];
        assert_eq!(mb.symbol_count, 1);
        assert_eq!(mb.line_count, 50);
        assert_eq!(mb.fan_in, 1, "mod_a calls into mod_b");
        assert_eq!(mb.fan_out, 0, "mod_b does not call external modules");
    }

    // -----------------------------------------------------------------------
    // Test: Complexity flags
    // -----------------------------------------------------------------------

    #[test]
    fn complexity_flags_high_cyclomatic() {
        let sym_id = SymbolId(1);
        let mut sym_metrics = HashMap::new();
        sym_metrics.insert(
            sym_id,
            SymbolMetrics::with_fan(sym_id, 20, 50, 3, 0, 0),
        );

        let mod_metrics = HashMap::new();
        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);

        let cc_flags: Vec<_> = flags
            .iter()
            .filter(|f| f.kind == ComplexityFlagKind::HighCyclomaticComplexity)
            .collect();
        assert_eq!(cc_flags.len(), 1);
        assert_eq!(cc_flags[0].value, 20);
        assert_eq!(cc_flags[0].threshold, 15);
        assert_eq!(cc_flags[0].target, FlagTarget::Symbol(sym_id));
    }

    #[test]
    fn complexity_flags_large_function() {
        let sym_id = SymbolId(1);
        let mut sym_metrics = HashMap::new();
        sym_metrics.insert(
            sym_id,
            SymbolMetrics::with_fan(sym_id, 5, 150, 3, 0, 0),
        );

        let mod_metrics = HashMap::new();
        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);

        let lf_flags: Vec<_> = flags
            .iter()
            .filter(|f| f.kind == ComplexityFlagKind::LargeFunction)
            .collect();
        assert_eq!(lf_flags.len(), 1);
        assert_eq!(lf_flags[0].value, 150);
        assert_eq!(lf_flags[0].threshold, 100);
    }

    #[test]
    fn complexity_flags_large_module() {
        let mod_id = ModuleId(1);
        let mut mod_metrics = HashMap::new();
        mod_metrics.insert(mod_id, ModuleMetrics::new(mod_id, 55, 1000, 0, 0));

        let sym_metrics = HashMap::new();
        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);

        let lm_flags: Vec<_> = flags
            .iter()
            .filter(|f| f.kind == ComplexityFlagKind::LargeModule)
            .collect();
        assert_eq!(lm_flags.len(), 1);
        assert_eq!(lm_flags[0].value, 55);
        assert_eq!(lm_flags[0].threshold, 50);
    }

    // -----------------------------------------------------------------------
    // Test: Function with 50 lines produces no flag, 150 lines produces flag
    // -----------------------------------------------------------------------

    #[test]
    fn complexity_flags_function_line_count_boundary() {
        let sym_below = SymbolId(1);
        let sym_above = SymbolId(2);
        let mut sym_metrics = HashMap::new();
        // 50 lines: at threshold boundary (not exceeding 100), no flag expected
        sym_metrics.insert(
            sym_below,
            SymbolMetrics::with_fan(sym_below, 1, 50, 2, 0, 0),
        );
        // 150 lines: exceeds threshold of 100, flag expected
        sym_metrics.insert(
            sym_above,
            SymbolMetrics::with_fan(sym_above, 1, 150, 2, 0, 0),
        );

        let mod_metrics = HashMap::new();
        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);

        // sym_below (50 lines) should produce no flags at all
        let below_flags: Vec<_> = flags
            .iter()
            .filter(|f| f.target == FlagTarget::Symbol(sym_below))
            .collect();
        assert!(
            below_flags.is_empty(),
            "function with 50 lines should not produce any flag"
        );

        // sym_above (150 lines) should produce a LargeFunction flag
        let above_flags: Vec<_> = flags
            .iter()
            .filter(|f| {
                f.target == FlagTarget::Symbol(sym_above)
                    && f.kind == ComplexityFlagKind::LargeFunction
            })
            .collect();
        assert_eq!(
            above_flags.len(),
            1,
            "function with 150 lines should produce exactly one LargeFunction flag"
        );
        assert_eq!(above_flags[0].value, 150);
        assert_eq!(above_flags[0].threshold, 100);
    }

    // -----------------------------------------------------------------------
    // Test: Module with 60 symbols produces LargeModule flag
    // -----------------------------------------------------------------------

    #[test]
    fn complexity_flags_module_60_symbols_large_module() {
        let mod_id = ModuleId(42);
        let mut mod_metrics = HashMap::new();
        mod_metrics.insert(mod_id, ModuleMetrics::new(mod_id, 60, 2000, 0, 0));

        let sym_metrics = HashMap::new();
        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);

        let lm_flags: Vec<_> = flags
            .iter()
            .filter(|f| f.kind == ComplexityFlagKind::LargeModule)
            .collect();
        assert_eq!(lm_flags.len(), 1);
        assert_eq!(lm_flags[0].value, 60);
        assert_eq!(lm_flags[0].threshold, 50);
        assert_eq!(lm_flags[0].target, FlagTarget::Module(mod_id));
    }

    // -----------------------------------------------------------------------
    // Test: Verify flag fields are populated correctly (value and threshold)
    // -----------------------------------------------------------------------

    #[test]
    fn complexity_flags_contain_correct_value_and_threshold() {
        let sym_id = SymbolId(10);
        let mod_id = ModuleId(20);

        let mut sym_metrics = HashMap::new();
        // CC = 20 (threshold 15), lines = 200 (threshold 100)
        sym_metrics.insert(
            sym_id,
            SymbolMetrics::with_fan(sym_id, 20, 200, 5, 0, 0),
        );

        let mut mod_metrics = HashMap::new();
        // symbol_count = 60 (threshold 50), fan_in = 25 (threshold 20), fan_out = 18 (threshold 15)
        mod_metrics.insert(mod_id, ModuleMetrics::new(mod_id, 60, 3000, 25, 18));

        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);

        // Should have 5 flags total: CC, LargeFunction, LargeModule, HighFanIn, HighFanOut
        assert_eq!(flags.len(), 5, "expected 5 flags total for all thresholds exceeded");

        // Verify each flag's value and threshold
        let cc = flags.iter().find(|f| f.kind == ComplexityFlagKind::HighCyclomaticComplexity).unwrap();
        assert_eq!(cc.value, 20);
        assert_eq!(cc.threshold, 15);

        let lf = flags.iter().find(|f| f.kind == ComplexityFlagKind::LargeFunction).unwrap();
        assert_eq!(lf.value, 200);
        assert_eq!(lf.threshold, 100);

        let lm = flags.iter().find(|f| f.kind == ComplexityFlagKind::LargeModule).unwrap();
        assert_eq!(lm.value, 60);
        assert_eq!(lm.threshold, 50);

        let fi = flags.iter().find(|f| f.kind == ComplexityFlagKind::HighFanIn).unwrap();
        assert_eq!(fi.value, 25);
        assert_eq!(fi.threshold, 20);

        let fo = flags.iter().find(|f| f.kind == ComplexityFlagKind::HighFanOut).unwrap();
        assert_eq!(fo.value, 18);
        assert_eq!(fo.threshold, 15);
    }

    #[test]
    fn complexity_flags_below_threshold_produce_no_flags() {
        let sym_id = SymbolId(1);
        let mut sym_metrics = HashMap::new();
        sym_metrics.insert(
            sym_id,
            SymbolMetrics::with_fan(sym_id, 5, 50, 3, 0, 0),
        );

        let mod_id = ModuleId(1);
        let mut mod_metrics = HashMap::new();
        mod_metrics.insert(mod_id, ModuleMetrics::new(mod_id, 10, 200, 5, 5));

        let flags = generate_complexity_flags(&sym_metrics, &mod_metrics);
        assert!(flags.is_empty(), "values below thresholds should not produce flags");
    }

    // -----------------------------------------------------------------------
    // Test: Empty module metrics
    // -----------------------------------------------------------------------

    #[test]
    fn empty_module_all_zeros() {
        let mod_id = ModuleId(1);
        let mut modules = HashMap::new();
        let mod_info = ModuleInfo::new(
            mod_id,
            "empty_mod",
            ModulePath::new("crate::empty_mod"),
            None,
            None,
        );
        modules.insert(mod_id, mod_info);

        let symbols = HashMap::new();
        let sym_metrics = HashMap::new();
        let call_graph_data = CallGraphData {
            callers: HashMap::new(),
            callees: HashMap::new(),
        };

        let mod_metrics = compute_module_metrics(
            &modules,
            &symbols,
            &sym_metrics,
            &call_graph_data,
        );

        let m = &mod_metrics[&mod_id];
        assert_eq!(m.symbol_count, 0);
        assert_eq!(m.line_count, 0);
        assert_eq!(m.fan_in, 0);
        assert_eq!(m.fan_out, 0);
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_test_symbol(
        id: SymbolId,
        name: &str,
        start_line: u32,
        end_line: u32,
        signature: Option<&str>,
    ) -> Symbol {
        let file_id = FileId(0);
        Symbol {
            id,
            name: name.to_owned(),
            kind: SymbolKind::Function,
            module_id: ModuleId(0),
            file_id,
            span: SourceSpan::new(file_id, start_line, 0, end_line, 0),
            visibility: Visibility::Public,
            signature: signature.map(|s| s.to_owned()),
            attributes: SymbolAttributes::empty(),
        }
    }
}
