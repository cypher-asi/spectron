//! Graph algorithm utilities for Spectron architecture graphs.
//!
//! This module provides common graph traversal and analysis functions
//! built on top of `petgraph`. All functions operate on [`ArchGraph`]
//! (i.e., `DiGraph<GraphNode, GraphEdge>`) instances.

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::graph::NodeIndex;
use petgraph::visit::EdgeRef;
use petgraph::Direction;

use spectron_core::ArchGraph;

// ---------------------------------------------------------------------------
// find_paths
// ---------------------------------------------------------------------------

/// Find all paths from `from` to `to` using BFS, with a maximum depth limit.
///
/// Returns a vector of paths, where each path is a vector of [`NodeIndex`]
/// values starting at `from` and ending at `to`. If no path exists within
/// the depth limit, an empty vector is returned.
///
/// The search is breadth-first, so shorter paths are discovered first.
/// Self-loops and cycles are handled by never revisiting a node within
/// a single path (no repeated vertices per path).
pub fn find_paths(
    graph: &ArchGraph,
    from: NodeIndex,
    to: NodeIndex,
    max_depth: usize,
) -> Vec<Vec<NodeIndex>> {
    let mut results = Vec::new();

    // BFS queue holds partial paths.
    let mut queue: VecDeque<Vec<NodeIndex>> = VecDeque::new();
    queue.push_back(vec![from]);

    while let Some(path) = queue.pop_front() {
        let current = *path.last().expect("path should never be empty");

        // If we reached the target, record the path.
        if current == to && path.len() > 1 {
            results.push(path);
            continue;
        }

        // If we have reached the depth limit, do not extend further.
        // Depth is measured as number of edges, which is path.len() - 1.
        if path.len() - 1 >= max_depth {
            continue;
        }

        // Extend the path through each outgoing neighbor.
        for neighbor in graph.neighbors_directed(current, Direction::Outgoing) {
            // Avoid revisiting nodes already in this path (prevents infinite loops).
            // Exception: allow `to` even if it equals `from` (for paths of length > 0).
            if neighbor == to || !path.contains(&neighbor) {
                let mut new_path = path.clone();
                new_path.push(neighbor);
                queue.push_back(new_path);
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// descendants
// ---------------------------------------------------------------------------

/// Get all descendants of a node using DFS traversal.
///
/// Returns all nodes reachable from `node` via outgoing edges,
/// **not** including `node` itself. The order is depth-first.
pub fn descendants(graph: &ArchGraph, node: NodeIndex) -> Vec<NodeIndex> {
    let mut visited = HashSet::new();
    let mut result = Vec::new();
    let mut stack = Vec::new();

    // Seed with the starting node (but do not include it in the result).
    visited.insert(node);
    stack.push(node);

    while let Some(current) = stack.pop() {
        for neighbor in graph.neighbors_directed(current, Direction::Outgoing) {
            if visited.insert(neighbor) {
                result.push(neighbor);
                stack.push(neighbor);
            }
        }
    }

    result
}

// ---------------------------------------------------------------------------
// reachable_subgraph
// ---------------------------------------------------------------------------

/// Extract the subgraph reachable from a set of root nodes.
///
/// Performs a DFS from each root, collecting all reachable nodes (including
/// the roots themselves). Returns a new [`ArchGraph`] containing only those
/// nodes and the edges between them.
///
/// Node weights and edge weights are cloned from the original graph.
pub fn reachable_subgraph(graph: &ArchGraph, roots: &[NodeIndex]) -> ArchGraph {
    // Collect all reachable nodes.
    let mut reachable = HashSet::new();
    let mut stack: Vec<NodeIndex> = Vec::new();

    for &root in roots {
        if reachable.insert(root) {
            stack.push(root);
        }
    }

    while let Some(current) = stack.pop() {
        for neighbor in graph.neighbors_directed(current, Direction::Outgoing) {
            if reachable.insert(neighbor) {
                stack.push(neighbor);
            }
        }
    }

    // Build the new graph.
    let mut subgraph = ArchGraph::new();
    let mut index_map: HashMap<NodeIndex, NodeIndex> = HashMap::new();

    // Add nodes.
    for &old_idx in &reachable {
        let new_idx = subgraph.add_node(graph[old_idx].clone());
        index_map.insert(old_idx, new_idx);
    }

    // Add edges (only between reachable nodes).
    for &old_idx in &reachable {
        for edge_ref in graph.edges_directed(old_idx, Direction::Outgoing) {
            let target = edge_ref.target();
            if reachable.contains(&target) {
                let new_src = index_map[&old_idx];
                let new_tgt = index_map[&target];
                subgraph.add_edge(new_src, new_tgt, edge_ref.weight().clone());
            }
        }
    }

    subgraph
}

// ---------------------------------------------------------------------------
// components
// ---------------------------------------------------------------------------

/// Compute connected components of the graph (treating edges as undirected).
///
/// Returns a vector of components, where each component is a vector of
/// [`NodeIndex`] values belonging to that component. Uses a simple
/// union-find / BFS approach over the undirected view of the graph.
pub fn components(graph: &ArchGraph) -> Vec<Vec<NodeIndex>> {
    let mut visited = HashSet::new();
    let mut result = Vec::new();

    for node in graph.node_indices() {
        if visited.contains(&node) {
            continue;
        }

        // BFS from this unvisited node to find its component.
        let mut component = Vec::new();
        let mut queue = VecDeque::new();
        visited.insert(node);
        queue.push_back(node);

        while let Some(current) = queue.pop_front() {
            component.push(current);

            // Follow outgoing edges.
            for neighbor in graph.neighbors_directed(current, Direction::Outgoing) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }

            // Follow incoming edges (treat as undirected).
            for neighbor in graph.neighbors_directed(current, Direction::Incoming) {
                if visited.insert(neighbor) {
                    queue.push_back(neighbor);
                }
            }
        }

        result.push(component);
    }

    result
}

// ---------------------------------------------------------------------------
// DataFlowInfo
// ---------------------------------------------------------------------------

/// Basic data flow metadata for a call edge.
///
/// Records the parameter/return value flow between a caller and callee.
/// This is attached as side-table metadata on call graph edges rather than
/// being a separate graph structure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DataFlowInfo {
    /// The calling function.
    pub caller: spectron_core::SymbolId,
    /// The called function.
    pub callee: spectron_core::SymbolId,
    /// Number of arguments passed from caller to callee.
    pub argument_count: usize,
    /// Whether the callee returns a value used by the caller.
    pub returns_value: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use spectron_core::{GraphEdge, GraphNode, RelationshipKind, SymbolId};

    /// Helper: build a linear graph A -> B -> C -> D.
    fn build_linear_graph() -> (ArchGraph, NodeIndex, NodeIndex, NodeIndex, NodeIndex) {
        let mut graph = ArchGraph::new();
        let a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        let c = graph.add_node(GraphNode::Symbol(SymbolId(2)));
        let d = graph.add_node(GraphNode::Symbol(SymbolId(3)));

        graph.add_edge(a, b, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(b, c, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(c, d, GraphEdge::new(RelationshipKind::Calls, 1.0));

        (graph, a, b, c, d)
    }

    // -------------------------------------------------------------------
    // find_paths tests
    // -------------------------------------------------------------------

    #[test]
    fn find_paths_linear_graph() {
        let (graph, a, _b, _c, d) = build_linear_graph();
        let paths = find_paths(&graph, a, d, 5);
        assert_eq!(paths.len(), 1, "expected exactly one path from A to D");
        assert_eq!(paths[0], vec![a, _b, _c, d]);
    }

    #[test]
    fn find_paths_no_path() {
        let (graph, _a, _b, _c, d) = build_linear_graph();
        // D has no outgoing edges, so no path from D to A.
        let paths = find_paths(&graph, d, _a, 10);
        assert!(paths.is_empty(), "expected no path from D to A");
    }

    #[test]
    fn find_paths_depth_limit_too_small() {
        let (graph, a, _b, _c, d) = build_linear_graph();
        // Path A->B->C->D has depth 3, so max_depth=2 should find nothing.
        let paths = find_paths(&graph, a, d, 2);
        assert!(
            paths.is_empty(),
            "expected no path when max_depth is too small"
        );
    }

    #[test]
    fn find_paths_adjacent() {
        let (graph, a, b, _c, _d) = build_linear_graph();
        let paths = find_paths(&graph, a, b, 1);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], vec![a, b]);
    }

    #[test]
    fn find_paths_multiple_paths() {
        // Build a diamond: A -> B -> D, A -> C -> D
        let mut graph = ArchGraph::new();
        let a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        let c = graph.add_node(GraphNode::Symbol(SymbolId(2)));
        let d = graph.add_node(GraphNode::Symbol(SymbolId(3)));

        graph.add_edge(a, b, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(a, c, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(b, d, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(c, d, GraphEdge::new(RelationshipKind::Calls, 1.0));

        let paths = find_paths(&graph, a, d, 5);
        assert_eq!(paths.len(), 2, "expected two paths through diamond");

        // Both paths should start with A and end with D.
        for path in &paths {
            assert_eq!(*path.first().unwrap(), a);
            assert_eq!(*path.last().unwrap(), d);
        }
    }

    #[test]
    fn find_paths_same_node() {
        let (graph, a, _b, _c, _d) = build_linear_graph();
        // from == to with no self-loop: should return no paths
        // (we require path.len() > 1, i.e., at least one edge).
        let paths = find_paths(&graph, a, a, 5);
        assert!(paths.is_empty(), "expected no trivial self-path");
    }

    // -------------------------------------------------------------------
    // descendants tests
    // -------------------------------------------------------------------

    #[test]
    fn descendants_from_root() {
        let (graph, a, b, c, d) = build_linear_graph();
        let desc = descendants(&graph, a);
        assert_eq!(desc.len(), 3, "A has 3 descendants: B, C, D");
        assert!(desc.contains(&b));
        assert!(desc.contains(&c));
        assert!(desc.contains(&d));
    }

    #[test]
    fn descendants_from_middle() {
        let (graph, _a, _b, c, d) = build_linear_graph();
        let desc = descendants(&graph, c);
        assert_eq!(desc.len(), 1, "C has 1 descendant: D");
        assert!(desc.contains(&d));
    }

    #[test]
    fn descendants_from_leaf() {
        let (graph, _a, _b, _c, d) = build_linear_graph();
        let desc = descendants(&graph, d);
        assert!(desc.is_empty(), "D has no descendants");
    }

    #[test]
    fn descendants_does_not_include_self() {
        let (graph, a, _b, _c, _d) = build_linear_graph();
        let desc = descendants(&graph, a);
        assert!(!desc.contains(&a), "should not include the starting node");
    }

    #[test]
    fn descendants_with_cycle() {
        // A -> B -> C -> A (cycle). Descendants of A should be {B, C}.
        let mut graph = ArchGraph::new();
        let a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        let c = graph.add_node(GraphNode::Symbol(SymbolId(2)));
        graph.add_edge(a, b, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(b, c, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(c, a, GraphEdge::new(RelationshipKind::Calls, 1.0));

        let desc = descendants(&graph, a);
        assert_eq!(desc.len(), 2);
        assert!(desc.contains(&b));
        assert!(desc.contains(&c));
    }

    // -------------------------------------------------------------------
    // reachable_subgraph tests
    // -------------------------------------------------------------------

    #[test]
    fn reachable_subgraph_from_middle() {
        let (graph, _a, b, _c, _d) = build_linear_graph();
        let sub = reachable_subgraph(&graph, &[b]);

        // B, C, D are reachable from B. A is not.
        assert_eq!(sub.node_count(), 3, "subgraph should have 3 nodes (B, C, D)");
        assert_eq!(sub.edge_count(), 2, "subgraph should have 2 edges (B->C, C->D)");

        // Verify all nodes are Symbol nodes with IDs 1, 2, 3.
        let ids: HashSet<_> = sub
            .node_indices()
            .map(|i| match &sub[i] {
                GraphNode::Symbol(id) => id.0,
                _ => panic!("unexpected node type"),
            })
            .collect();
        assert!(ids.contains(&1), "should contain B (SymbolId 1)");
        assert!(ids.contains(&2), "should contain C (SymbolId 2)");
        assert!(ids.contains(&3), "should contain D (SymbolId 3)");
        assert!(!ids.contains(&0), "should NOT contain A (SymbolId 0)");
    }

    #[test]
    fn reachable_subgraph_from_leaf() {
        let (graph, _a, _b, _c, d) = build_linear_graph();
        let sub = reachable_subgraph(&graph, &[d]);

        assert_eq!(sub.node_count(), 1, "only D is reachable from D");
        assert_eq!(sub.edge_count(), 0, "no edges from D");
    }

    #[test]
    fn reachable_subgraph_multiple_roots() {
        // A -> B, C -> D (two disconnected chains).
        let mut graph = ArchGraph::new();
        let a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        let c = graph.add_node(GraphNode::Symbol(SymbolId(2)));
        let d = graph.add_node(GraphNode::Symbol(SymbolId(3)));
        graph.add_edge(a, b, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(c, d, GraphEdge::new(RelationshipKind::Calls, 1.0));

        let sub = reachable_subgraph(&graph, &[a, c]);
        assert_eq!(sub.node_count(), 4, "all 4 nodes reachable from roots A and C");
        assert_eq!(sub.edge_count(), 2);
    }

    // -------------------------------------------------------------------
    // components tests
    // -------------------------------------------------------------------

    #[test]
    fn components_single_connected() {
        let (graph, _a, _b, _c, _d) = build_linear_graph();
        let comps = components(&graph);
        assert_eq!(comps.len(), 1, "linear graph is a single component");
        assert_eq!(comps[0].len(), 4);
    }

    #[test]
    fn components_disconnected_graph() {
        // Two disconnected pairs: A -> B, C -> D
        let mut graph = ArchGraph::new();
        let a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        let c = graph.add_node(GraphNode::Symbol(SymbolId(2)));
        let d = graph.add_node(GraphNode::Symbol(SymbolId(3)));
        graph.add_edge(a, b, GraphEdge::new(RelationshipKind::Calls, 1.0));
        graph.add_edge(c, d, GraphEdge::new(RelationshipKind::Calls, 1.0));

        let comps = components(&graph);
        assert_eq!(comps.len(), 2, "expected 2 connected components");

        // Each component should have 2 nodes.
        let mut sizes: Vec<usize> = comps.iter().map(|c| c.len()).collect();
        sizes.sort();
        assert_eq!(sizes, vec![2, 2]);
    }

    #[test]
    fn components_isolated_nodes() {
        let mut graph = ArchGraph::new();
        let _a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let _b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        let _c = graph.add_node(GraphNode::Symbol(SymbolId(2)));

        let comps = components(&graph);
        assert_eq!(comps.len(), 3, "3 isolated nodes = 3 components");
    }

    #[test]
    fn components_empty_graph() {
        let graph = ArchGraph::new();
        let comps = components(&graph);
        assert!(comps.is_empty(), "empty graph has no components");
    }

    #[test]
    fn components_treats_edges_as_undirected() {
        // A -> B (directed). Undirected view: they are in the same component.
        let mut graph = ArchGraph::new();
        let a = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let b = graph.add_node(GraphNode::Symbol(SymbolId(1)));
        graph.add_edge(a, b, GraphEdge::new(RelationshipKind::Calls, 1.0));

        let comps = components(&graph);
        assert_eq!(comps.len(), 1, "A and B should be in the same component");
        assert_eq!(comps[0].len(), 2);
    }

    // -------------------------------------------------------------------
    // DataFlowInfo tests
    // -------------------------------------------------------------------

    #[test]
    fn data_flow_info_construction() {
        let info = DataFlowInfo {
            caller: SymbolId(1),
            callee: SymbolId(2),
            argument_count: 3,
            returns_value: true,
        };

        assert_eq!(info.caller, SymbolId(1));
        assert_eq!(info.callee, SymbolId(2));
        assert_eq!(info.argument_count, 3);
        assert!(info.returns_value);
    }

    #[test]
    fn data_flow_info_clone_and_eq() {
        let info = DataFlowInfo {
            caller: SymbolId(10),
            callee: SymbolId(20),
            argument_count: 0,
            returns_value: false,
        };
        let cloned = info.clone();
        assert_eq!(info, cloned);
    }
}
