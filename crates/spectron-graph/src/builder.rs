//! Structure graph construction and graph index.
//!
//! This module implements the core graph construction logic for Spectron.
//! It consumes a [`LoadResult`] (project structure) and a [`ParseResult`]
//! (symbols and relationships) and produces a [`GraphSet`] containing:
//!
//! - A **structure graph** with all crates, modules, files, and symbols as nodes,
//!   connected by Contains, Imports, Implements, DependsOn, and References edges.
//! - A **call graph** (stub for now, populated by a future task).
//! - A **graph index** for O(1) lookups from domain IDs to graph node indices.

use std::collections::{HashMap, HashSet};

use petgraph::graph::NodeIndex;
use tracing;

use spectron_core::{
    ArchGraph, CrateId, FileId, GraphEdge, GraphNode, ModuleId,
    RelationshipKind, SymbolId, SymbolKind,
};
use spectron_loader::LoadResult;
use spectron_parser::ParseResult;

use crate::cfg::ControlFlowGraph;

// ---------------------------------------------------------------------------
// Synthetic "unresolved" module
// ---------------------------------------------------------------------------

/// Sentinel [`ModuleId`] used for the synthetic "unresolved" module.
///
/// Orphan symbols -- those whose `module_id` does not match any module in the
/// [`LoadResult`] -- are attached to this synthetic module rather than being
/// left disconnected in the structure graph.
pub const UNRESOLVED_MODULE_ID: ModuleId = ModuleId(u64::MAX);

// ---------------------------------------------------------------------------
// Edge weight constants
// ---------------------------------------------------------------------------

/// Weight for `Contains` edges. Higher weight keeps children close to parents
/// in layout algorithms.
const CONTAINS_WEIGHT: f32 = 2.0;

/// Default weight for all other edge types.
const DEFAULT_WEIGHT: f32 = 1.0;

// ---------------------------------------------------------------------------
// GraphSet
// ---------------------------------------------------------------------------

/// The complete set of graph representations produced by the graph builder.
pub struct GraphSet {
    /// Full architecture graph (crates, modules, files, symbols, all edges).
    pub structure_graph: ArchGraph,
    /// Call-only graph (functions/methods and call edges).
    pub call_graph: ArchGraph,
    /// Precomputed callers/callees side tables for the call graph.
    pub call_graph_data: CallGraphData,
    /// Per-function control flow graphs.
    pub control_flow_graphs: HashMap<SymbolId, ControlFlowGraph>,
    /// Index for efficient lookups from domain IDs to graph node indices.
    pub index: GraphIndex,
}

// ---------------------------------------------------------------------------
// CallGraphData
// ---------------------------------------------------------------------------

/// Precomputed caller/callee side tables for the call graph.
///
/// For each callable symbol (Function or Method) present in the call graph,
/// this structure provides the list of direct callers (incoming call edges)
/// and callees (outgoing call edges). These are derived from the call graph
/// and stored for efficient inspector panel lookups.
pub struct CallGraphData {
    /// For each symbol in the call graph, the list of symbols that call it.
    pub callers: HashMap<SymbolId, Vec<SymbolId>>,
    /// For each symbol in the call graph, the list of symbols it calls.
    pub callees: HashMap<SymbolId, Vec<SymbolId>>,
}

// ---------------------------------------------------------------------------
// GraphIndex
// ---------------------------------------------------------------------------

/// O(1) lookup index from domain entity IDs to their corresponding
/// [`NodeIndex`] values in the structure and call graphs.
pub struct GraphIndex {
    /// Map from CrateId to node index in structure_graph.
    pub crate_nodes: HashMap<CrateId, NodeIndex>,
    /// Map from ModuleId to node index in structure_graph.
    pub module_nodes: HashMap<ModuleId, NodeIndex>,
    /// Map from FileId to node index in structure_graph.
    pub file_nodes: HashMap<FileId, NodeIndex>,
    /// Map from SymbolId to node index in structure_graph.
    pub symbol_nodes: HashMap<SymbolId, NodeIndex>,
    /// Map from SymbolId to node index in call_graph.
    pub call_nodes: HashMap<SymbolId, NodeIndex>,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Build all graph representations from analysis inputs.
///
/// Constructs a structure graph with nodes for every crate, module, file, and
/// symbol, connected by Contains, Imports, Implements, DependsOn, and
/// References edges. Also builds a call graph containing only callable symbols
/// (Function and Method) connected by Calls edges, with precomputed
/// callers/callees side tables. Finally, builds a GraphIndex for O(1) domain
/// ID lookups into both graphs.
///
/// Control flow graphs are initialized as empty stubs; they will be populated
/// by future tasks.
pub fn build_graphs(load_result: &LoadResult, parse_result: &ParseResult) -> GraphSet {
    let mut structure_graph = ArchGraph::new();
    let mut index = GraphIndex {
        crate_nodes: HashMap::new(),
        module_nodes: HashMap::new(),
        file_nodes: HashMap::new(),
        symbol_nodes: HashMap::new(),
        call_nodes: HashMap::new(),
    };

    // -- Step 1: Add nodes --------------------------------------------------

    // Add crate nodes.
    for krate in &load_result.crates {
        let node_idx = structure_graph.add_node(GraphNode::Crate(krate.id));
        index.crate_nodes.insert(krate.id, node_idx);
    }

    // Add module nodes.
    for module in &load_result.modules {
        let node_idx = structure_graph.add_node(GraphNode::Module(module.id));
        index.module_nodes.insert(module.id, node_idx);
    }

    // Add file nodes.
    for file in &load_result.files {
        let node_idx = structure_graph.add_node(GraphNode::File(file.id));
        index.file_nodes.insert(file.id, node_idx);
    }

    // Add symbol nodes.
    for symbol in &parse_result.symbols {
        let node_idx = structure_graph.add_node(GraphNode::Symbol(symbol.id));
        index.symbol_nodes.insert(symbol.id, node_idx);
    }

    // -- Step 2: Add Contains edges -----------------------------------------

    // Crate -> Module (root modules: those listed in crate.module_ids whose
    // parent is None).
    let module_map: HashMap<ModuleId, &spectron_core::ModuleInfo> = load_result
        .modules
        .iter()
        .map(|m| (m.id, m))
        .collect();

    for krate in &load_result.crates {
        let crate_idx = index.crate_nodes[&krate.id];
        for &module_id in &krate.module_ids {
            if let Some(module_info) = module_map.get(&module_id) {
                if module_info.parent.is_none() {
                    // Root module -- connect crate -> module.
                    if let Some(&mod_idx) = index.module_nodes.get(&module_id) {
                        add_contains_edge(&mut structure_graph, crate_idx, mod_idx);
                    }
                }
            }
        }
    }

    // Module -> Module (parent -> child).
    for module in &load_result.modules {
        if let Some(&parent_idx) = index.module_nodes.get(&module.id) {
            for &child_id in &module.children {
                if let Some(&child_idx) = index.module_nodes.get(&child_id) {
                    // parent_idx is actually the current module; we need to
                    // add edge from current module to child.
                    add_contains_edge(&mut structure_graph, parent_idx, child_idx);
                }
            }
        }
    }

    // Module -> Symbol (each symbol belongs to a module).
    // Orphan symbols (whose module_id does not match any known module) are
    // attached to a synthetic "unresolved" module per spec section 10.
    let mut unresolved_module_idx: Option<NodeIndex> = None;

    for symbol in &parse_result.symbols {
        if let Some(&mod_idx) = index.module_nodes.get(&symbol.module_id) {
            if let Some(&sym_idx) = index.symbol_nodes.get(&symbol.id) {
                add_contains_edge(&mut structure_graph, mod_idx, sym_idx);
            }
        } else {
            // Lazily create the synthetic "unresolved" module on first orphan.
            let unresolved_idx = *unresolved_module_idx.get_or_insert_with(|| {
                tracing::warn!(
                    "creating synthetic 'unresolved' module for orphan symbols"
                );
                let idx = structure_graph
                    .add_node(GraphNode::Module(UNRESOLVED_MODULE_ID));
                index.module_nodes.insert(UNRESOLVED_MODULE_ID, idx);
                idx
            });

            tracing::warn!(
                symbol_id = symbol.id.0,
                module_id = symbol.module_id.0,
                "symbol has no matching module; attaching to synthetic 'unresolved' module"
            );

            if let Some(&sym_idx) = index.symbol_nodes.get(&symbol.id) {
                add_contains_edge(&mut structure_graph, unresolved_idx, sym_idx);
            }
        }
    }

    // -- Step 3: Add relationship-based edges --------------------------------

    // Build a set of existing edges for deduplication. The key is
    // (source_node_index, target_node_index, relationship_kind).
    let mut edge_set: HashSet<(NodeIndex, NodeIndex, &'static str)> = HashSet::new();

    for rel in &parse_result.relationships {
        let kind_key = relationship_kind_key(&rel.kind);

        // For symbol-to-symbol relationships we need both source and target in
        // the index.
        let source_idx = match index.symbol_nodes.get(&rel.source) {
            Some(&idx) => idx,
            None => {
                tracing::debug!(
                    source = rel.source.0,
                    kind = %rel.kind,
                    "skipping relationship: source symbol not in graph"
                );
                continue;
            }
        };
        let target_idx = match index.symbol_nodes.get(&rel.target) {
            Some(&idx) => idx,
            None => {
                tracing::debug!(
                    target = rel.target.0,
                    kind = %rel.kind,
                    "skipping relationship: target symbol not in graph"
                );
                continue;
            }
        };

        // Deduplicate: only add one edge per (source, target, kind) triple.
        let edge_key = (source_idx, target_idx, kind_key);
        if !edge_set.insert(edge_key) {
            continue;
        }

        match rel.kind {
            RelationshipKind::Imports => {
                structure_graph.add_edge(
                    source_idx,
                    target_idx,
                    GraphEdge::new(RelationshipKind::Imports, DEFAULT_WEIGHT),
                );
            }
            RelationshipKind::Implements => {
                structure_graph.add_edge(
                    source_idx,
                    target_idx,
                    GraphEdge::new(RelationshipKind::Implements, DEFAULT_WEIGHT),
                );
            }
            RelationshipKind::References => {
                structure_graph.add_edge(
                    source_idx,
                    target_idx,
                    GraphEdge::new(RelationshipKind::References, DEFAULT_WEIGHT),
                );
            }
            RelationshipKind::Calls => {
                structure_graph.add_edge(
                    source_idx,
                    target_idx,
                    GraphEdge::new(RelationshipKind::Calls, DEFAULT_WEIGHT),
                );
            }
            // Contains and DependsOn are handled structurally, not from
            // Relationship records.
            RelationshipKind::Contains | RelationshipKind::DependsOn => {}
        }
    }

    // -- Step 4: DependsOn edges (crate -> crate) ----------------------------
    //
    // For each crate, look at its `dependencies` list (populated from
    // Cargo.toml) and create DependsOn edges to any matching crate node
    // in the graph. Dependencies that don't match a known crate in the
    // project (i.e. external dependencies like `serde`) are skipped.

    // Build a lookup from crate name to CrateId(s). Multiple crate targets
    // (e.g. library + binary) may share the same name, so we use a Vec.
    let crate_name_to_ids: HashMap<&str, Vec<CrateId>> = {
        let mut map: HashMap<&str, Vec<CrateId>> = HashMap::new();
        for krate in &load_result.crates {
            map.entry(krate.name.as_str()).or_default().push(krate.id);
        }
        map
    };

    // Track existing DependsOn edges to avoid duplicates (e.g. when both
    // lib and bin targets of the same crate declare the same dependency).
    let mut depends_on_edges: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();

    for krate in &load_result.crates {
        let source_idx = match index.crate_nodes.get(&krate.id) {
            Some(&idx) => idx,
            None => continue,
        };

        for dep_name in &krate.dependencies {
            // Normalize: Cargo.toml uses hyphens but Rust crate names use
            // underscores. Try both the original name and the underscore form.
            let normalized_name = dep_name.replace('-', "_");

            let target_ids = crate_name_to_ids
                .get(dep_name.as_str())
                .or_else(|| crate_name_to_ids.get(normalized_name.as_str()));

            let target_ids = match target_ids {
                Some(ids) => ids,
                None => {
                    tracing::debug!(
                        crate_name = krate.name.as_str(),
                        dependency = dep_name.as_str(),
                        "DependsOn: dependency not found among project crates (likely external)"
                    );
                    continue;
                }
            };

            for &target_crate_id in target_ids {
                // Don't create self-dependency edges.
                if target_crate_id == krate.id {
                    continue;
                }

                let target_idx = match index.crate_nodes.get(&target_crate_id) {
                    Some(&idx) => idx,
                    None => continue,
                };

                // Deduplicate.
                if depends_on_edges.insert((source_idx, target_idx)) {
                    structure_graph.add_edge(
                        source_idx,
                        target_idx,
                        GraphEdge::new(RelationshipKind::DependsOn, DEFAULT_WEIGHT),
                    );
                }
            }
        }
    }

    // -- Step 5: Build call graph -------------------------------------------
    //
    // The call graph contains only callable symbols (Function and Method)
    // and only Calls edges between them. Self-recursive calls are allowed
    // (self-loop edges). Edges are deduplicated per (source, target) pair.

    let mut call_graph = ArchGraph::new();
    let mut call_nodes: HashMap<SymbolId, NodeIndex> = HashMap::new();

    // Build a lookup from SymbolId to SymbolKind for filtering.
    let symbol_kind_map: HashMap<SymbolId, &SymbolKind> = parse_result
        .symbols
        .iter()
        .map(|s| (s.id, &s.kind))
        .collect();

    // Add nodes for callable symbols only.
    for symbol in &parse_result.symbols {
        if symbol.kind == SymbolKind::Function || symbol.kind == SymbolKind::Method {
            let node_idx = call_graph.add_node(GraphNode::Symbol(symbol.id));
            call_nodes.insert(symbol.id, node_idx);
        }
    }

    // Add Calls edges between callable symbols. Deduplicate per (source, target).
    let mut call_edge_set: HashSet<(NodeIndex, NodeIndex)> = HashSet::new();

    for rel in &parse_result.relationships {
        if rel.kind != RelationshipKind::Calls {
            continue;
        }

        // Both source and target must be callable symbols present in the call graph.
        let source_idx = match call_nodes.get(&rel.source) {
            Some(&idx) => idx,
            None => {
                // Source is not a callable symbol or not in the graph.
                // Check if the symbol exists at all for a better log message.
                if symbol_kind_map.contains_key(&rel.source) {
                    tracing::debug!(
                        source = rel.source.0,
                        "call graph: skipping Calls edge - source is not a callable symbol"
                    );
                } else {
                    tracing::warn!(
                        source = rel.source.0,
                        "call graph: skipping Calls edge - unresolved source symbol"
                    );
                }
                continue;
            }
        };

        let target_idx = match call_nodes.get(&rel.target) {
            Some(&idx) => idx,
            None => {
                if symbol_kind_map.contains_key(&rel.target) {
                    tracing::debug!(
                        target = rel.target.0,
                        "call graph: skipping Calls edge - target is not a callable symbol"
                    );
                } else {
                    tracing::warn!(
                        target = rel.target.0,
                        "call graph: skipping Calls edge - unresolved target symbol"
                    );
                }
                continue;
            }
        };

        // Deduplicate: one edge per (source, target) pair. Self-loops allowed.
        if call_edge_set.insert((source_idx, target_idx)) {
            call_graph.add_edge(
                source_idx,
                target_idx,
                GraphEdge::new(RelationshipKind::Calls, DEFAULT_WEIGHT),
            );
        }
    }

    // -- Step 6: Precompute callers/callees side tables ----------------------

    let mut callers: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();
    let mut callees: HashMap<SymbolId, Vec<SymbolId>> = HashMap::new();

    // Initialize empty entries for all callable symbols so that every symbol
    // in the call graph has an entry even if it has no callers or callees.
    for &sym_id in call_nodes.keys() {
        callers.entry(sym_id).or_default();
        callees.entry(sym_id).or_default();
    }

    // Walk the call graph edges to populate the tables.
    for edge_idx in call_graph.edge_indices() {
        if let Some((src_node, tgt_node)) = call_graph.edge_endpoints(edge_idx) {
            // Resolve node indices back to SymbolIds.
            if let (GraphNode::Symbol(src_id), GraphNode::Symbol(tgt_id)) =
                (&call_graph[src_node], &call_graph[tgt_node])
            {
                callees.entry(*src_id).or_default().push(*tgt_id);
                callers.entry(*tgt_id).or_default().push(*src_id);
            }
        }
    }

    let call_graph_data = CallGraphData { callers, callees };

    // Store call_nodes in the index.
    index.call_nodes = call_nodes;

    // -- Assemble the GraphSet -----------------------------------------------

    GraphSet {
        structure_graph,
        call_graph,
        call_graph_data,
        control_flow_graphs: HashMap::new(),
        index,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Add a `Contains` edge between two nodes with the standard contains weight.
fn add_contains_edge(graph: &mut ArchGraph, parent: NodeIndex, child: NodeIndex) {
    graph.add_edge(
        parent,
        child,
        GraphEdge::new(RelationshipKind::Contains, CONTAINS_WEIGHT),
    );
}

/// Return a static string key for a relationship kind, used for edge
/// deduplication.
fn relationship_kind_key(kind: &RelationshipKind) -> &'static str {
    match kind {
        RelationshipKind::Contains => "Contains",
        RelationshipKind::Imports => "Imports",
        RelationshipKind::Calls => "Calls",
        RelationshipKind::References => "References",
        RelationshipKind::Implements => "Implements",
        RelationshipKind::DependsOn => "DependsOn",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use spectron_core::{
        CrateId, CrateInfo, CrateType, FileId, FileInfo, GraphNode, ModuleId,
        ModuleInfo, ModulePath, ProjectInfo, Relationship, RelationshipKind,
        SourceSpan, Symbol, SymbolAttributes, SymbolId, SymbolKind, Visibility,
    };
    use spectron_loader::LoadResult;
    use spectron_parser::ParseResult;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Build a minimal LoadResult and ParseResult for testing.
    ///
    /// Creates:
    /// - 1 crate (crate_id=0)
    /// - 2 modules: root (mod_id=0, parent=None) and child (mod_id=1, parent=Some(0))
    /// - 1 file (file_id=0)
    /// - 3 symbols: sym0 in mod0, sym1 in mod1, sym2 in mod1
    fn build_test_inputs() -> (LoadResult, ParseResult) {
        let crate_id = CrateId(0);
        let mod0 = ModuleId(0);
        let mod1 = ModuleId(1);
        let file_id = FileId(0);
        let sym0 = SymbolId(100);
        let sym1 = SymbolId(101);
        let sym2 = SymbolId(102);

        let mut root_module = ModuleInfo::new(
            mod0,
            "my_crate",
            ModulePath::new("my_crate"),
            Some(PathBuf::from("src/lib.rs")),
            None,
        );
        root_module.children.push(mod1);
        root_module.symbol_ids.push(sym0);

        let mut child_module = ModuleInfo::new(
            mod1,
            "child",
            ModulePath::new("my_crate::child"),
            Some(PathBuf::from("src/child.rs")),
            Some(mod0),
        );
        child_module.symbol_ids.push(sym1);
        child_module.symbol_ids.push(sym2);

        let mut krate = CrateInfo::new(crate_id, "my_crate", "/tmp/my_crate", CrateType::Library);
        krate.module_ids.push(mod0);
        krate.module_ids.push(mod1);

        let project = ProjectInfo::new("my_project", "/tmp/my_project", false);
        let file = FileInfo::new(file_id, "src/lib.rs", "abc123", 100);

        let load_result = LoadResult {
            project,
            crates: vec![krate],
            modules: vec![root_module, child_module],
            files: vec![file],
        };

        let span = SourceSpan::new(file_id, 1, 0, 1, 10);
        let symbols = vec![
            Symbol {
                id: sym0,
                name: "foo".to_owned(),
                kind: SymbolKind::Function,
                module_id: mod0,
                file_id,
                span: span.clone(),
                visibility: Visibility::Public,
                signature: Some("fn foo()".to_owned()),
                attributes: SymbolAttributes::empty(),
            },
            Symbol {
                id: sym1,
                name: "bar".to_owned(),
                kind: SymbolKind::Function,
                module_id: mod1,
                file_id,
                span: span.clone(),
                visibility: Visibility::Public,
                signature: Some("fn bar()".to_owned()),
                attributes: SymbolAttributes::empty(),
            },
            Symbol {
                id: sym2,
                name: "Baz".to_owned(),
                kind: SymbolKind::Struct,
                module_id: mod1,
                file_id,
                span: span.clone(),
                visibility: Visibility::Public,
                signature: None,
                attributes: SymbolAttributes::empty(),
            },
        ];

        let parse_result = ParseResult {
            symbols,
            relationships: Vec::new(),
            errors: Vec::new(),
        };

        (load_result, parse_result)
    }

    // -----------------------------------------------------------------------
    // Test: node counts (1 crate, 2 modules, 1 file, 3 symbols = 7 nodes)
    // -----------------------------------------------------------------------

    #[test]
    fn structure_graph_node_count() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        // 1 crate + 2 modules + 1 file + 3 symbols = 7 nodes
        assert_eq!(
            g.node_count(),
            7,
            "expected 7 nodes (1 crate + 2 modules + 1 file + 3 symbols)"
        );
    }

    // -----------------------------------------------------------------------
    // Test: GraphIndex lookups return correct NodeIndex values
    // -----------------------------------------------------------------------

    #[test]
    fn graph_index_lookups_are_correct() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        // Verify crate node lookup.
        let crate_idx = idx.crate_nodes[&CrateId(0)];
        assert_eq!(g[crate_idx], GraphNode::Crate(CrateId(0)));

        // Verify module node lookups.
        let mod0_idx = idx.module_nodes[&ModuleId(0)];
        assert_eq!(g[mod0_idx], GraphNode::Module(ModuleId(0)));

        let mod1_idx = idx.module_nodes[&ModuleId(1)];
        assert_eq!(g[mod1_idx], GraphNode::Module(ModuleId(1)));

        // Verify file node lookup.
        let file_idx = idx.file_nodes[&FileId(0)];
        assert_eq!(g[file_idx], GraphNode::File(FileId(0)));

        // Verify symbol node lookups.
        for &sym_id in &[SymbolId(100), SymbolId(101), SymbolId(102)] {
            let sym_idx = idx.symbol_nodes[&sym_id];
            assert_eq!(g[sym_idx], GraphNode::Symbol(sym_id));
        }
    }

    // -----------------------------------------------------------------------
    // Test: Contains edges point parent -> child (never reverse)
    // -----------------------------------------------------------------------

    #[test]
    fn contains_edges_point_parent_to_child() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        // Collect all Contains edges as (source_node, target_node) pairs.
        let contains_edges: Vec<(GraphNode, GraphNode)> = g
            .edge_indices()
            .filter(|&e| g[e].kind == RelationshipKind::Contains)
            .map(|e| {
                let (src, tgt) = g.edge_endpoints(e).unwrap();
                (g[src].clone(), g[tgt].clone())
            })
            .collect();

        // Verify that no Contains edge goes from child to parent.
        // A parent is always a "higher-level" entity: Crate > Module > Symbol.
        for (src, tgt) in &contains_edges {
            match (src, tgt) {
                (GraphNode::Crate(_), GraphNode::Module(_)) => { /* valid */ }
                (GraphNode::Module(_), GraphNode::Module(_)) => { /* valid */ }
                (GraphNode::Module(_), GraphNode::Symbol(_)) => { /* valid */ }
                (src, tgt) => {
                    panic!(
                        "unexpected Contains edge direction: {} -> {}",
                        src, tgt
                    );
                }
            }
        }

        // Verify specific Contains edges exist:
        // Crate(0) -> Module(0) (root module)
        let crate_idx = idx.crate_nodes[&CrateId(0)];
        let mod0_idx = idx.module_nodes[&ModuleId(0)];
        assert!(
            has_contains_edge(g, crate_idx, mod0_idx),
            "expected Contains edge from Crate(0) to Module(0)"
        );

        // Module(0) -> Module(1) (parent -> child module)
        let mod1_idx = idx.module_nodes[&ModuleId(1)];
        assert!(
            has_contains_edge(g, mod0_idx, mod1_idx),
            "expected Contains edge from Module(0) to Module(1)"
        );

        // Module(0) -> Symbol(100)
        let sym0_idx = idx.symbol_nodes[&SymbolId(100)];
        assert!(
            has_contains_edge(g, mod0_idx, sym0_idx),
            "expected Contains edge from Module(0) to Symbol(100)"
        );

        // Module(1) -> Symbol(101)
        let sym1_idx = idx.symbol_nodes[&SymbolId(101)];
        assert!(
            has_contains_edge(g, mod1_idx, sym1_idx),
            "expected Contains edge from Module(1) to Symbol(101)"
        );

        // Module(1) -> Symbol(102)
        let sym2_idx = idx.symbol_nodes[&SymbolId(102)];
        assert!(
            has_contains_edge(g, mod1_idx, sym2_idx),
            "expected Contains edge from Module(1) to Symbol(102)"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Contains edge weight is 2.0
    // -----------------------------------------------------------------------

    #[test]
    fn contains_edges_have_weight_2() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        for edge_idx in g.edge_indices() {
            let edge = &g[edge_idx];
            if edge.kind == RelationshipKind::Contains {
                assert!(
                    (edge.weight - CONTAINS_WEIGHT).abs() < f32::EPSILON,
                    "Contains edge should have weight {}, got {}",
                    CONTAINS_WEIGHT,
                    edge.weight
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test: Relationship-based edges (Imports, References, etc.)
    // -----------------------------------------------------------------------

    #[test]
    fn relationship_edges_are_created() {
        let (load, mut parse) = build_test_inputs();

        // Add an Imports relationship: sym0 imports sym1
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Imports,
        ));

        // Add a References relationship: sym1 references sym2
        parse.relationships.push(Relationship::new(
            SymbolId(101),
            SymbolId(102),
            RelationshipKind::References,
        ));

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        // Check Imports edge exists.
        let sym0_idx = idx.symbol_nodes[&SymbolId(100)];
        let sym1_idx = idx.symbol_nodes[&SymbolId(101)];
        assert!(
            has_edge_of_kind(g, sym0_idx, sym1_idx, &RelationshipKind::Imports),
            "expected Imports edge from sym0 to sym1"
        );

        // Check References edge exists.
        let sym2_idx = idx.symbol_nodes[&SymbolId(102)];
        assert!(
            has_edge_of_kind(g, sym1_idx, sym2_idx, &RelationshipKind::References),
            "expected References edge from sym1 to sym2"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Duplicate edges are deduplicated
    // -----------------------------------------------------------------------

    #[test]
    fn duplicate_relationship_edges_are_deduplicated() {
        let (load, mut parse) = build_test_inputs();

        // Add the same Imports relationship twice.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Imports,
        ));
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Imports,
        ));

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        let sym0_idx = idx.symbol_nodes[&SymbolId(100)];
        let sym1_idx = idx.symbol_nodes[&SymbolId(101)];

        // Count Imports edges between sym0 and sym1.
        let import_edge_count = g
            .edges_connecting(sym0_idx, sym1_idx)
            .filter(|e| e.weight().kind == RelationshipKind::Imports)
            .count();
        assert_eq!(
            import_edge_count, 1,
            "duplicate Imports edges should be deduplicated"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Non-relationship edges have default weight 1.0
    // -----------------------------------------------------------------------

    #[test]
    fn non_contains_edges_have_default_weight() {
        let (load, mut parse) = build_test_inputs();

        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Imports,
        ));

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        for edge_idx in g.edge_indices() {
            let edge = &g[edge_idx];
            if edge.kind != RelationshipKind::Contains {
                assert!(
                    (edge.weight - DEFAULT_WEIGHT).abs() < f32::EPSILON,
                    "non-Contains edge should have weight {}, got {}",
                    DEFAULT_WEIGHT,
                    edge.weight
                );
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test: Empty crate (no symbols, no modules)
    // -----------------------------------------------------------------------

    #[test]
    fn empty_crate_has_crate_node_with_no_children() {
        let crate_id = CrateId(0);
        let krate = CrateInfo::new(crate_id, "empty_crate", "/tmp/empty", CrateType::Library);
        let project = ProjectInfo::new("empty", "/tmp/empty", false);

        let load = LoadResult {
            project,
            crates: vec![krate],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        // Should have exactly 1 node (the crate).
        assert_eq!(g.node_count(), 1);
        assert_eq!(g.edge_count(), 0);

        // The crate node should exist in the index.
        assert!(graph_set.index.crate_nodes.contains_key(&crate_id));
    }

    // -----------------------------------------------------------------------
    // Test: All symbols in parse result have nodes in structure graph
    // -----------------------------------------------------------------------

    #[test]
    fn every_symbol_has_a_graph_node() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);

        for symbol in &parse.symbols {
            assert!(
                graph_set.index.symbol_nodes.contains_key(&symbol.id),
                "symbol {} should have a node in the structure graph",
                symbol.id
            );
        }
    }

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    /// Check if a Contains edge exists from `parent` to `child`.
    fn has_contains_edge(graph: &ArchGraph, parent: NodeIndex, child: NodeIndex) -> bool {
        graph
            .edges_connecting(parent, child)
            .any(|e| e.weight().kind == RelationshipKind::Contains)
    }

    /// Check if an edge of a specific kind exists from `source` to `target`.
    fn has_edge_of_kind(
        graph: &ArchGraph,
        source: NodeIndex,
        target: NodeIndex,
        kind: &RelationshipKind,
    ) -> bool {
        graph
            .edges_connecting(source, target)
            .any(|e| &e.weight().kind == kind)
    }

    // -----------------------------------------------------------------------
    // Test: Orphan symbols are attached to synthetic "unresolved" module
    // -----------------------------------------------------------------------

    #[test]
    fn orphan_symbols_attached_to_unresolved_module() {
        // Create a load result with one crate and one module (mod0).
        let crate_id = CrateId(0);
        let mod0 = ModuleId(0);
        let file_id = FileId(0);
        let sym_ok = SymbolId(200);
        let sym_orphan1 = SymbolId(201);
        let sym_orphan2 = SymbolId(202);

        let root_module = ModuleInfo::new(
            mod0,
            "my_crate",
            ModulePath::new("my_crate"),
            Some(PathBuf::from("src/lib.rs")),
            None,
        );

        let mut krate = CrateInfo::new(crate_id, "my_crate", "/tmp/my_crate", CrateType::Library);
        krate.module_ids.push(mod0);

        let project = ProjectInfo::new("my_project", "/tmp/my_project", false);
        let file = FileInfo::new(file_id, "src/lib.rs", "abc123", 100);

        let load = LoadResult {
            project,
            crates: vec![krate],
            modules: vec![root_module],
            files: vec![file],
        };

        let span = SourceSpan::new(file_id, 1, 0, 1, 10);

        // sym_ok belongs to mod0 (valid), sym_orphan1 and sym_orphan2 belong
        // to ModuleId(999) which does not exist in the load result.
        let nonexistent_module = ModuleId(999);
        let parse = ParseResult {
            symbols: vec![
                Symbol {
                    id: sym_ok,
                    name: "good_fn".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: None,
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: sym_orphan1,
                    name: "orphan_fn".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: nonexistent_module,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: None,
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: sym_orphan2,
                    name: "OrphanStruct".to_owned(),
                    kind: SymbolKind::Struct,
                    module_id: nonexistent_module,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: None,
                    attributes: SymbolAttributes::empty(),
                },
            ],
            relationships: Vec::new(),
            errors: Vec::new(),
        };

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        // The synthetic "unresolved" module should exist in the index.
        assert!(
            idx.module_nodes.contains_key(&UNRESOLVED_MODULE_ID),
            "expected synthetic unresolved module in the graph index"
        );

        let unresolved_idx = idx.module_nodes[&UNRESOLVED_MODULE_ID];

        // Verify the node is a Module with the sentinel ID.
        assert_eq!(
            g[unresolved_idx],
            GraphNode::Module(UNRESOLVED_MODULE_ID),
        );

        // Both orphan symbols should have Contains edges from the unresolved
        // module.
        let orphan1_idx = idx.symbol_nodes[&sym_orphan1];
        let orphan2_idx = idx.symbol_nodes[&sym_orphan2];

        assert!(
            has_contains_edge(g, unresolved_idx, orphan1_idx),
            "expected Contains edge from unresolved module to orphan symbol 1"
        );
        assert!(
            has_contains_edge(g, unresolved_idx, orphan2_idx),
            "expected Contains edge from unresolved module to orphan symbol 2"
        );

        // The non-orphan symbol should still be attached to its real module.
        let sym_ok_idx = idx.symbol_nodes[&sym_ok];
        let mod0_idx = idx.module_nodes[&mod0];
        assert!(
            has_contains_edge(g, mod0_idx, sym_ok_idx),
            "expected Contains edge from Module(0) to non-orphan symbol"
        );

        // Node count: 1 crate + 1 real module + 1 unresolved module +
        // 1 file + 3 symbols = 7
        assert_eq!(
            g.node_count(),
            7,
            "expected 7 nodes (1 crate + 2 modules [incl. unresolved] + 1 file + 3 symbols)"
        );
    }

    // -----------------------------------------------------------------------
    // Test: No unresolved module created when there are no orphans
    // -----------------------------------------------------------------------

    #[test]
    fn no_unresolved_module_when_no_orphans() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);

        assert!(
            !graph_set.index.module_nodes.contains_key(&UNRESOLVED_MODULE_ID),
            "unresolved module should not be created when there are no orphan symbols"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Orphan symbols are still indexed in symbol_nodes
    // -----------------------------------------------------------------------

    #[test]
    fn orphan_symbols_are_indexed() {
        let crate_id = CrateId(0);
        let file_id = FileId(0);
        let sym_orphan = SymbolId(300);

        let krate = CrateInfo::new(crate_id, "my_crate", "/tmp/my_crate", CrateType::Library);
        let project = ProjectInfo::new("my_project", "/tmp/my_project", false);
        let file = FileInfo::new(file_id, "src/lib.rs", "abc123", 100);

        let load = LoadResult {
            project,
            crates: vec![krate],
            modules: Vec::new(),
            files: vec![file],
        };

        let span = SourceSpan::new(file_id, 1, 0, 1, 10);
        let parse = ParseResult {
            symbols: vec![Symbol {
                id: sym_orphan,
                name: "orphan".to_owned(),
                kind: SymbolKind::Function,
                module_id: ModuleId(999),
                file_id,
                span,
                visibility: Visibility::Public,
                signature: None,
                attributes: SymbolAttributes::empty(),
            }],
            relationships: Vec::new(),
            errors: Vec::new(),
        };

        let graph_set = build_graphs(&load, &parse);

        // The orphan symbol should be in the symbol index.
        assert!(
            graph_set.index.symbol_nodes.contains_key(&sym_orphan),
            "orphan symbol should be indexed in symbol_nodes"
        );

        // It should have a Contains edge from the unresolved module.
        let g = &graph_set.structure_graph;
        let unresolved_idx = graph_set.index.module_nodes[&UNRESOLVED_MODULE_ID];
        let sym_idx = graph_set.index.symbol_nodes[&sym_orphan];
        assert!(
            has_contains_edge(g, unresolved_idx, sym_idx),
            "orphan symbol should be connected to unresolved module"
        );
    }

    // -----------------------------------------------------------------------
    // Test: DependsOn edges are created between crates with matching deps
    // -----------------------------------------------------------------------

    #[test]
    fn depends_on_edges_created_for_intra_project_dependencies() {
        // Create two crates: crate_a depends on crate_b.
        let crate_a_id = CrateId(10);
        let crate_b_id = CrateId(20);

        let mut crate_a = CrateInfo::new(
            crate_a_id,
            "crate_a",
            "/tmp/project/crate_a",
            CrateType::Library,
        );
        crate_a.dependencies = vec!["crate_b".to_owned()];

        let crate_b = CrateInfo::new(
            crate_b_id,
            "crate_b",
            "/tmp/project/crate_b",
            CrateType::Library,
        );

        let project = ProjectInfo::new("test_project", "/tmp/project", true);

        let load = LoadResult {
            project,
            crates: vec![crate_a, crate_b],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        let a_idx = idx.crate_nodes[&crate_a_id];
        let b_idx = idx.crate_nodes[&crate_b_id];

        assert!(
            has_edge_of_kind(g, a_idx, b_idx, &RelationshipKind::DependsOn),
            "expected DependsOn edge from crate_a to crate_b"
        );

        // No reverse edge should exist.
        assert!(
            !has_edge_of_kind(g, b_idx, a_idx, &RelationshipKind::DependsOn),
            "should not have DependsOn edge from crate_b to crate_a"
        );
    }

    // -----------------------------------------------------------------------
    // Test: DependsOn edges use default weight (1.0)
    // -----------------------------------------------------------------------

    #[test]
    fn depends_on_edges_have_default_weight() {
        let crate_a_id = CrateId(10);
        let crate_b_id = CrateId(20);

        let mut crate_a = CrateInfo::new(
            crate_a_id,
            "crate_a",
            "/tmp/project/crate_a",
            CrateType::Library,
        );
        crate_a.dependencies = vec!["crate_b".to_owned()];

        let crate_b = CrateInfo::new(
            crate_b_id,
            "crate_b",
            "/tmp/project/crate_b",
            CrateType::Library,
        );

        let project = ProjectInfo::new("test_project", "/tmp/project", true);

        let load = LoadResult {
            project,
            crates: vec![crate_a, crate_b],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        let a_idx = idx.crate_nodes[&crate_a_id];
        let b_idx = idx.crate_nodes[&crate_b_id];

        let edge = g
            .edges_connecting(a_idx, b_idx)
            .find(|e| e.weight().kind == RelationshipKind::DependsOn)
            .expect("DependsOn edge should exist");

        assert!(
            (edge.weight().weight - DEFAULT_WEIGHT).abs() < f32::EPSILON,
            "DependsOn edge should have default weight {}, got {}",
            DEFAULT_WEIGHT,
            edge.weight().weight
        );
    }

    // -----------------------------------------------------------------------
    // Test: External dependencies are silently skipped
    // -----------------------------------------------------------------------

    #[test]
    fn external_dependencies_do_not_create_edges() {
        let crate_a_id = CrateId(10);

        let mut crate_a = CrateInfo::new(
            crate_a_id,
            "crate_a",
            "/tmp/project/crate_a",
            CrateType::Library,
        );
        // "serde" is external and not in the project.
        crate_a.dependencies = vec!["serde".to_owned(), "tokio".to_owned()];

        let project = ProjectInfo::new("test_project", "/tmp/project", false);

        let load = LoadResult {
            project,
            crates: vec![crate_a],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        // No DependsOn edges should exist.
        let depends_on_count = g
            .edge_indices()
            .filter(|&e| g[e].kind == RelationshipKind::DependsOn)
            .count();
        assert_eq!(
            depends_on_count, 0,
            "external dependencies should not produce DependsOn edges"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Hyphenated dependency names match underscore crate names
    // -----------------------------------------------------------------------

    #[test]
    fn depends_on_normalizes_hyphens_to_underscores() {
        let crate_a_id = CrateId(10);
        let crate_b_id = CrateId(20);

        let mut crate_a = CrateInfo::new(
            crate_a_id,
            "crate_a",
            "/tmp/project/crate_a",
            CrateType::Library,
        );
        // Dependency declared with hyphens, but the crate name uses underscores.
        crate_a.dependencies = vec!["crate-b".to_owned()];

        let crate_b = CrateInfo::new(
            crate_b_id,
            "crate_b",
            "/tmp/project/crate_b",
            CrateType::Library,
        );

        let project = ProjectInfo::new("test_project", "/tmp/project", true);

        let load = LoadResult {
            project,
            crates: vec![crate_a, crate_b],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        let a_idx = idx.crate_nodes[&crate_a_id];
        let b_idx = idx.crate_nodes[&crate_b_id];

        assert!(
            has_edge_of_kind(g, a_idx, b_idx, &RelationshipKind::DependsOn),
            "expected DependsOn edge even with hyphenated dep name"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Self-dependencies are not created
    // -----------------------------------------------------------------------

    #[test]
    fn depends_on_skips_self_dependency() {
        let crate_a_id = CrateId(10);

        let mut crate_a = CrateInfo::new(
            crate_a_id,
            "crate_a",
            "/tmp/project/crate_a",
            CrateType::Library,
        );
        // A crate listing itself as a dependency should not create a self-loop.
        crate_a.dependencies = vec!["crate_a".to_owned()];

        let project = ProjectInfo::new("test_project", "/tmp/project", false);

        let load = LoadResult {
            project,
            crates: vec![crate_a],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        let depends_on_count = g
            .edge_indices()
            .filter(|&e| g[e].kind == RelationshipKind::DependsOn)
            .count();
        assert_eq!(
            depends_on_count, 0,
            "self-dependency should not produce a DependsOn edge"
        );
    }

    // -----------------------------------------------------------------------
    // Test: DependsOn edges are deduplicated across targets
    // -----------------------------------------------------------------------

    #[test]
    fn depends_on_edges_are_deduplicated() {
        // Both lib and bin targets of crate_a depend on crate_b. Only one
        // DependsOn edge should be created per (source_crate, target_crate)
        // pair. In this case, lib and bin are separate crate nodes, so each
        // gets its own edge.
        let lib_id = CrateId(10);
        let bin_id = CrateId(11);
        let dep_id = CrateId(20);

        let mut crate_lib = CrateInfo::new(
            lib_id,
            "my_crate",
            "/tmp/project/my_crate",
            CrateType::Library,
        );
        crate_lib.dependencies = vec!["dep_crate".to_owned()];

        let mut crate_bin = CrateInfo::new(
            bin_id,
            "my_crate",
            "/tmp/project/my_crate",
            CrateType::Binary,
        );
        crate_bin.dependencies = vec!["dep_crate".to_owned()];

        let dep_crate = CrateInfo::new(
            dep_id,
            "dep_crate",
            "/tmp/project/dep_crate",
            CrateType::Library,
        );

        let project = ProjectInfo::new("test_project", "/tmp/project", true);

        let load = LoadResult {
            project,
            crates: vec![crate_lib, crate_bin, dep_crate],
            modules: Vec::new(),
            files: Vec::new(),
        };
        let parse = ParseResult::new();

        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;
        let idx = &graph_set.index;

        let lib_idx = idx.crate_nodes[&lib_id];
        let bin_idx = idx.crate_nodes[&bin_id];
        let dep_idx = idx.crate_nodes[&dep_id];

        // Each target should have exactly one DependsOn edge to dep_crate.
        let lib_to_dep = g
            .edges_connecting(lib_idx, dep_idx)
            .filter(|e| e.weight().kind == RelationshipKind::DependsOn)
            .count();
        assert_eq!(lib_to_dep, 1, "lib -> dep should have exactly 1 DependsOn edge");

        let bin_to_dep = g
            .edges_connecting(bin_idx, dep_idx)
            .filter(|e| e.weight().kind == RelationshipKind::DependsOn)
            .count();
        assert_eq!(bin_to_dep, 1, "bin -> dep should have exactly 1 DependsOn edge");
    }

    // -----------------------------------------------------------------------
    // Test: No DependsOn edges when crates have no dependencies
    // -----------------------------------------------------------------------

    #[test]
    fn no_depends_on_edges_when_no_dependencies() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let g = &graph_set.structure_graph;

        let depends_on_count = g
            .edge_indices()
            .filter(|&e| g[e].kind == RelationshipKind::DependsOn)
            .count();
        assert_eq!(
            depends_on_count, 0,
            "no DependsOn edges when crates have empty dependencies"
        );
    }

    // =======================================================================
    // Call Graph Tests
    // =======================================================================

    // -----------------------------------------------------------------------
    // Test: Call graph contains only Function/Method symbols
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_contains_only_callable_symbols() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;

        // From build_test_inputs: sym0 = Function, sym1 = Function, sym2 = Struct
        // Call graph should contain sym0 and sym1 but NOT sym2.
        assert_eq!(
            cg.node_count(),
            2,
            "call graph should contain 2 callable symbols (2 Functions)"
        );

        // Verify all nodes are Symbol nodes for callable types.
        for node_idx in cg.node_indices() {
            match &cg[node_idx] {
                GraphNode::Symbol(id) => {
                    assert!(
                        graph_set.index.call_nodes.contains_key(id),
                        "call graph node {} should be in call_nodes index",
                        id
                    );
                }
                other => panic!("unexpected node in call graph: {}", other),
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test: Call graph index maps callable symbols correctly
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_index_maps_callable_symbols() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let idx = &graph_set.index;

        // sym0 (Function) and sym1 (Function) should be in call_nodes.
        assert!(idx.call_nodes.contains_key(&SymbolId(100)));
        assert!(idx.call_nodes.contains_key(&SymbolId(101)));

        // sym2 (Struct) should NOT be in call_nodes.
        assert!(!idx.call_nodes.contains_key(&SymbolId(102)));

        // Verify the node indices point to the correct nodes.
        let call_idx_0 = idx.call_nodes[&SymbolId(100)];
        assert_eq!(cg[call_idx_0], GraphNode::Symbol(SymbolId(100)));

        let call_idx_1 = idx.call_nodes[&SymbolId(101)];
        assert_eq!(cg[call_idx_1], GraphNode::Symbol(SymbolId(101)));
    }

    // -----------------------------------------------------------------------
    // Test: Call graph edges from Calls relationships
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_has_calls_edges() {
        let (load, mut parse) = build_test_inputs();

        // sym0 (Function) calls sym1 (Function).
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let idx = &graph_set.index;

        assert_eq!(cg.edge_count(), 1, "call graph should have 1 Calls edge");

        let src_idx = idx.call_nodes[&SymbolId(100)];
        let tgt_idx = idx.call_nodes[&SymbolId(101)];

        assert!(
            has_edge_of_kind(cg, src_idx, tgt_idx, &RelationshipKind::Calls),
            "expected Calls edge from sym0 to sym1 in call graph"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call graph ignores non-Calls relationships
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_ignores_non_calls_relationships() {
        let (load, mut parse) = build_test_inputs();

        // Add Imports and References relationships between callable symbols.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Imports,
        ));
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::References,
        ));

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;

        assert_eq!(
            cg.edge_count(),
            0,
            "call graph should have no edges from non-Calls relationships"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call graph excludes non-callable symbol targets
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_skips_calls_to_non_callable_symbols() {
        let (load, mut parse) = build_test_inputs();

        // sym0 (Function) "calls" sym2 (Struct) -- should be skipped.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(102),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;

        assert_eq!(
            cg.edge_count(),
            0,
            "call graph should skip Calls edges to non-callable symbols"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Self-recursive calls create self-loop edges
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_allows_self_recursive_calls() {
        let (load, mut parse) = build_test_inputs();

        // sym0 (Function) calls itself.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(100),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let idx = &graph_set.index;

        assert_eq!(cg.edge_count(), 1, "call graph should have 1 self-loop edge");

        let self_idx = idx.call_nodes[&SymbolId(100)];
        assert!(
            has_edge_of_kind(cg, self_idx, self_idx, &RelationshipKind::Calls),
            "expected self-loop Calls edge on sym0"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Duplicate Calls edges are deduplicated in call graph
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_deduplicates_edges() {
        let (load, mut parse) = build_test_inputs();

        // Add the same Calls relationship twice.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Calls,
        ));
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let idx = &graph_set.index;

        let src_idx = idx.call_nodes[&SymbolId(100)];
        let tgt_idx = idx.call_nodes[&SymbolId(101)];

        let calls_count = cg
            .edges_connecting(src_idx, tgt_idx)
            .filter(|e| e.weight().kind == RelationshipKind::Calls)
            .count();
        assert_eq!(
            calls_count, 1,
            "duplicate Calls edges should be deduplicated in call graph"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Callers/callees side tables are populated correctly
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_callers_callees_tables() {
        let (load, mut parse) = build_test_inputs();

        // sym0 calls sym1.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(101),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let data = &graph_set.call_graph_data;

        // sym0 callees should include sym1.
        let sym0_callees = &data.callees[&SymbolId(100)];
        assert_eq!(sym0_callees, &vec![SymbolId(101)]);

        // sym0 callers should be empty.
        let sym0_callers = &data.callers[&SymbolId(100)];
        assert!(sym0_callers.is_empty());

        // sym1 callers should include sym0.
        let sym1_callers = &data.callers[&SymbolId(101)];
        assert_eq!(sym1_callers, &vec![SymbolId(100)]);

        // sym1 callees should be empty.
        let sym1_callees = &data.callees[&SymbolId(101)];
        assert!(sym1_callees.is_empty());
    }

    // -----------------------------------------------------------------------
    // Test: Every callable symbol has entries in callers/callees tables
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_all_callable_symbols_have_table_entries() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let data = &graph_set.call_graph_data;
        let idx = &graph_set.index;

        // Every symbol in call_nodes should have entries in both tables.
        for sym_id in idx.call_nodes.keys() {
            assert!(
                data.callers.contains_key(sym_id),
                "callable symbol {} should have a callers entry",
                sym_id
            );
            assert!(
                data.callees.contains_key(sym_id),
                "callable symbol {} should have a callees entry",
                sym_id
            );
        }
    }

    // -----------------------------------------------------------------------
    // Test: Self-recursive calls appear in both callers and callees
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_self_recursive_in_callers_callees() {
        let (load, mut parse) = build_test_inputs();

        // sym0 calls itself.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(100),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let data = &graph_set.call_graph_data;

        // sym0 should appear in its own callers and callees.
        let sym0_callers = &data.callers[&SymbolId(100)];
        assert!(
            sym0_callers.contains(&SymbolId(100)),
            "self-recursive sym0 should be in its own callers"
        );

        let sym0_callees = &data.callees[&SymbolId(100)];
        assert!(
            sym0_callees.contains(&SymbolId(100)),
            "self-recursive sym0 should be in its own callees"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call graph is empty when no Calls relationships exist
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_empty_when_no_calls() {
        let (load, parse) = build_test_inputs();
        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;

        // Nodes exist (for callable symbols) but no edges.
        assert_eq!(cg.node_count(), 2, "callable symbols should have nodes");
        assert_eq!(cg.edge_count(), 0, "no Calls edges means no call graph edges");
    }

    // -----------------------------------------------------------------------
    // Test: Call graph with Method symbols
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_includes_method_symbols() {
        let crate_id = CrateId(0);
        let mod0 = ModuleId(0);
        let file_id = FileId(0);

        let root_module = ModuleInfo::new(
            mod0,
            "my_crate",
            ModulePath::new("my_crate"),
            Some(PathBuf::from("src/lib.rs")),
            None,
        );

        let mut krate = CrateInfo::new(crate_id, "my_crate", "/tmp/my_crate", CrateType::Library);
        krate.module_ids.push(mod0);

        let project = ProjectInfo::new("my_project", "/tmp/my_project", false);
        let file = FileInfo::new(file_id, "src/lib.rs", "abc123", 100);

        let load = LoadResult {
            project,
            crates: vec![krate],
            modules: vec![root_module],
            files: vec![file],
        };

        let span = SourceSpan::new(file_id, 1, 0, 1, 10);
        let fn_sym = SymbolId(200);
        let method_sym = SymbolId(201);
        let struct_sym = SymbolId(202);

        let parse = ParseResult {
            symbols: vec![
                Symbol {
                    id: fn_sym,
                    name: "do_thing".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn do_thing()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: method_sym,
                    name: "run".to_owned(),
                    kind: SymbolKind::Method,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn run(&self)".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: struct_sym,
                    name: "Runner".to_owned(),
                    kind: SymbolKind::Struct,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: None,
                    attributes: SymbolAttributes::empty(),
                },
            ],
            relationships: vec![
                // Function calls Method.
                Relationship::new(fn_sym, method_sym, RelationshipKind::Calls),
                // Method calls Function.
                Relationship::new(method_sym, fn_sym, RelationshipKind::Calls),
            ],
            errors: Vec::new(),
        };

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let idx = &graph_set.index;

        // Call graph has 2 nodes (Function + Method), not the Struct.
        assert_eq!(cg.node_count(), 2);
        assert_eq!(cg.edge_count(), 2);

        assert!(idx.call_nodes.contains_key(&fn_sym));
        assert!(idx.call_nodes.contains_key(&method_sym));
        assert!(!idx.call_nodes.contains_key(&struct_sym));

        // Verify edges.
        let fn_idx = idx.call_nodes[&fn_sym];
        let method_idx = idx.call_nodes[&method_sym];
        assert!(has_edge_of_kind(cg, fn_idx, method_idx, &RelationshipKind::Calls));
        assert!(has_edge_of_kind(cg, method_idx, fn_idx, &RelationshipKind::Calls));

        // Verify callers/callees.
        let data = &graph_set.call_graph_data;
        assert_eq!(data.callees[&fn_sym], vec![method_sym]);
        assert_eq!(data.callers[&fn_sym], vec![method_sym]);
        assert_eq!(data.callees[&method_sym], vec![fn_sym]);
        assert_eq!(data.callers[&method_sym], vec![fn_sym]);
    }

    // -----------------------------------------------------------------------
    // Test: Unresolved call targets are omitted with warning
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_skips_unresolved_targets() {
        let (load, mut parse) = build_test_inputs();

        // sym0 calls a non-existent symbol.
        parse.relationships.push(Relationship::new(
            SymbolId(100),
            SymbolId(999),
            RelationshipKind::Calls,
        ));

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;

        assert_eq!(
            cg.edge_count(),
            0,
            "call graph should omit edges with unresolved targets"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call chain A -> B -> C: 3 nodes, 2 edges, correct side tables
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_chain_a_calls_b_calls_c() {
        let crate_id = CrateId(0);
        let mod0 = ModuleId(0);
        let file_id = FileId(0);

        let root_module = ModuleInfo::new(
            mod0,
            "my_crate",
            ModulePath::new("my_crate"),
            Some(PathBuf::from("src/lib.rs")),
            None,
        );

        let mut krate = CrateInfo::new(crate_id, "my_crate", "/tmp/my_crate", CrateType::Library);
        krate.module_ids.push(mod0);

        let project = ProjectInfo::new("my_project", "/tmp/my_project", false);
        let file = FileInfo::new(file_id, "src/lib.rs", "abc123", 100);

        let load = LoadResult {
            project,
            crates: vec![krate],
            modules: vec![root_module],
            files: vec![file],
        };

        let span = SourceSpan::new(file_id, 1, 0, 1, 10);
        let fn_a = SymbolId(400);
        let fn_b = SymbolId(401);
        let fn_c = SymbolId(402);

        let parse = ParseResult {
            symbols: vec![
                Symbol {
                    id: fn_a,
                    name: "fn_a".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn fn_a()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: fn_b,
                    name: "fn_b".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn fn_b()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: fn_c,
                    name: "fn_c".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn fn_c()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
            ],
            relationships: vec![
                // A calls B, B calls C.
                Relationship::new(fn_a, fn_b, RelationshipKind::Calls),
                Relationship::new(fn_b, fn_c, RelationshipKind::Calls),
            ],
            errors: Vec::new(),
        };

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let idx = &graph_set.index;
        let data = &graph_set.call_graph_data;

        // 3 callable symbols -> 3 nodes.
        assert_eq!(cg.node_count(), 3, "expected 3 nodes in call graph (A, B, C)");

        // A->B and B->C -> 2 edges.
        assert_eq!(cg.edge_count(), 2, "expected 2 edges in call graph (A->B, B->C)");

        // Verify edges exist.
        let a_idx = idx.call_nodes[&fn_a];
        let b_idx = idx.call_nodes[&fn_b];
        let c_idx = idx.call_nodes[&fn_c];

        assert!(
            has_edge_of_kind(cg, a_idx, b_idx, &RelationshipKind::Calls),
            "expected Calls edge A -> B"
        );
        assert!(
            has_edge_of_kind(cg, b_idx, c_idx, &RelationshipKind::Calls),
            "expected Calls edge B -> C"
        );
        assert!(
            !has_edge_of_kind(cg, a_idx, c_idx, &RelationshipKind::Calls),
            "should NOT have transitive edge A -> C"
        );

        // Verify callers/callees side tables.
        // A: callees=[B], callers=[]
        assert_eq!(data.callees[&fn_a], vec![fn_b]);
        assert!(data.callers[&fn_a].is_empty());

        // B: callees=[C], callers=[A]
        assert_eq!(data.callees[&fn_b], vec![fn_c]);
        assert_eq!(data.callers[&fn_b], vec![fn_a]);

        // C: callees=[], callers=[B]
        assert!(data.callees[&fn_c].is_empty());
        assert_eq!(data.callers[&fn_c], vec![fn_b]);
    }

    // -----------------------------------------------------------------------
    // Test: Multiple callees from one caller
    // -----------------------------------------------------------------------

    #[test]
    fn call_graph_multiple_callees() {
        let crate_id = CrateId(0);
        let mod0 = ModuleId(0);
        let file_id = FileId(0);

        let root_module = ModuleInfo::new(
            mod0,
            "my_crate",
            ModulePath::new("my_crate"),
            Some(PathBuf::from("src/lib.rs")),
            None,
        );

        let mut krate = CrateInfo::new(crate_id, "my_crate", "/tmp/my_crate", CrateType::Library);
        krate.module_ids.push(mod0);

        let project = ProjectInfo::new("my_project", "/tmp/my_project", false);
        let file = FileInfo::new(file_id, "src/lib.rs", "abc123", 100);

        let load = LoadResult {
            project,
            crates: vec![krate],
            modules: vec![root_module],
            files: vec![file],
        };

        let span = SourceSpan::new(file_id, 1, 0, 1, 10);
        let caller = SymbolId(300);
        let callee_a = SymbolId(301);
        let callee_b = SymbolId(302);

        let parse = ParseResult {
            symbols: vec![
                Symbol {
                    id: caller,
                    name: "main".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn main()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: callee_a,
                    name: "helper_a".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn helper_a()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
                Symbol {
                    id: callee_b,
                    name: "helper_b".to_owned(),
                    kind: SymbolKind::Function,
                    module_id: mod0,
                    file_id,
                    span: span.clone(),
                    visibility: Visibility::Public,
                    signature: Some("fn helper_b()".to_owned()),
                    attributes: SymbolAttributes::empty(),
                },
            ],
            relationships: vec![
                Relationship::new(caller, callee_a, RelationshipKind::Calls),
                Relationship::new(caller, callee_b, RelationshipKind::Calls),
            ],
            errors: Vec::new(),
        };

        let graph_set = build_graphs(&load, &parse);
        let cg = &graph_set.call_graph;
        let data = &graph_set.call_graph_data;

        assert_eq!(cg.node_count(), 3);
        assert_eq!(cg.edge_count(), 2);

        // caller has 2 callees.
        let caller_callees = &data.callees[&caller];
        assert_eq!(caller_callees.len(), 2);
        assert!(caller_callees.contains(&callee_a));
        assert!(caller_callees.contains(&callee_b));

        // caller has 0 callers.
        assert!(data.callers[&caller].is_empty());

        // Each callee has 1 caller (the caller) and 0 callees.
        assert_eq!(data.callers[&callee_a], vec![caller]);
        assert_eq!(data.callers[&callee_b], vec![caller]);
        assert!(data.callees[&callee_a].is_empty());
        assert!(data.callees[&callee_b].is_empty());
    }
}
