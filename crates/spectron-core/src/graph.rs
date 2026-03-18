//! Relationship and graph types for the architecture graph.
//!
//! This module defines the types used to represent relationships between code
//! entities and the architecture graph that ties the entire analysis together.

use std::fmt;

use petgraph::graph::DiGraph;
use serde::{Deserialize, Serialize};

use crate::id::{CrateId, FileId, ModuleId, SymbolId};
use crate::symbol::SourceSpan;

// ---------------------------------------------------------------------------
// RelationshipKind
// ---------------------------------------------------------------------------

/// The kind of relationship between two code entities.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipKind {
    /// Module contains a child module or symbol.
    Contains,
    /// A use/import statement.
    Imports,
    /// Function A calls function B.
    Calls,
    /// Symbol A references symbol B.
    References,
    /// Type implements a trait.
    Implements,
    /// Crate A depends on crate B.
    DependsOn,
}

impl fmt::Display for RelationshipKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelationshipKind::Contains => write!(f, "Contains"),
            RelationshipKind::Imports => write!(f, "Imports"),
            RelationshipKind::Calls => write!(f, "Calls"),
            RelationshipKind::References => write!(f, "References"),
            RelationshipKind::Implements => write!(f, "Implements"),
            RelationshipKind::DependsOn => write!(f, "DependsOn"),
        }
    }
}

// ---------------------------------------------------------------------------
// Relationship
// ---------------------------------------------------------------------------

/// A directed relationship between two symbols.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Relationship {
    /// The source (origin) of the relationship.
    pub source: SymbolId,
    /// The target (destination) of the relationship.
    pub target: SymbolId,
    /// What kind of relationship this is.
    pub kind: RelationshipKind,
    /// Optional source span where this relationship originates (e.g. the call site).
    pub span: Option<SourceSpan>,
}

impl Relationship {
    /// Create a new relationship without a source span.
    pub fn new(source: SymbolId, target: SymbolId, kind: RelationshipKind) -> Self {
        Self {
            source,
            target,
            kind,
            span: None,
        }
    }

    /// Create a new relationship with a source span.
    pub fn with_span(
        source: SymbolId,
        target: SymbolId,
        kind: RelationshipKind,
        span: SourceSpan,
    ) -> Self {
        Self {
            source,
            target,
            kind,
            span: Some(span),
        }
    }
}

// ---------------------------------------------------------------------------
// GraphNode
// ---------------------------------------------------------------------------

/// A node in the architecture graph, representing one of several entity types.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GraphNode {
    /// A crate-level node.
    Crate(CrateId),
    /// A module-level node.
    Module(ModuleId),
    /// A file-level node.
    File(FileId),
    /// A symbol-level node.
    Symbol(SymbolId),
}

impl fmt::Display for GraphNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphNode::Crate(id) => write!(f, "Crate({})", id),
            GraphNode::Module(id) => write!(f, "Module({})", id),
            GraphNode::File(id) => write!(f, "File({})", id),
            GraphNode::Symbol(id) => write!(f, "Symbol({})", id),
        }
    }
}

// ---------------------------------------------------------------------------
// GraphEdge
// ---------------------------------------------------------------------------

/// An edge in the architecture graph.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphEdge {
    /// The kind of relationship this edge represents.
    pub kind: RelationshipKind,
    /// Weight for layout influence (higher weight = stronger pull in layout).
    pub weight: f32,
}

impl GraphEdge {
    /// Create a new graph edge with the given kind and weight.
    pub fn new(kind: RelationshipKind, weight: f32) -> Self {
        Self { kind, weight }
    }
}

// ---------------------------------------------------------------------------
// ArchGraph
// ---------------------------------------------------------------------------

/// The primary directed graph type used throughout Spectron.
///
/// Nodes are [`GraphNode`] variants (crate, module, file, or symbol) and edges
/// are [`GraphEdge`] values carrying a relationship kind and layout weight.
pub type ArchGraph = DiGraph<GraphNode, GraphEdge>;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdGenerator;

    #[test]
    fn relationship_kind_display() {
        assert_eq!(format!("{}", RelationshipKind::Contains), "Contains");
        assert_eq!(format!("{}", RelationshipKind::Imports), "Imports");
        assert_eq!(format!("{}", RelationshipKind::Calls), "Calls");
        assert_eq!(format!("{}", RelationshipKind::References), "References");
        assert_eq!(format!("{}", RelationshipKind::Implements), "Implements");
        assert_eq!(format!("{}", RelationshipKind::DependsOn), "DependsOn");
    }

    #[test]
    fn relationship_kind_equality() {
        assert_eq!(RelationshipKind::Calls, RelationshipKind::Calls);
        assert_ne!(RelationshipKind::Calls, RelationshipKind::Imports);
    }

    #[test]
    fn relationship_kind_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(RelationshipKind::Calls);
        set.insert(RelationshipKind::Calls);
        set.insert(RelationshipKind::Imports);
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn relationship_new() {
        let src = SymbolId(1);
        let tgt = SymbolId(2);
        let rel = Relationship::new(src, tgt, RelationshipKind::Calls);

        assert_eq!(rel.source, src);
        assert_eq!(rel.target, tgt);
        assert_eq!(rel.kind, RelationshipKind::Calls);
        assert!(rel.span.is_none());
    }

    #[test]
    fn relationship_with_span() {
        let src = SymbolId(1);
        let tgt = SymbolId(2);
        let span = SourceSpan::new(FileId(0), 10, 4, 10, 20);
        let rel = Relationship::with_span(src, tgt, RelationshipKind::Imports, span.clone());

        assert_eq!(rel.source, src);
        assert_eq!(rel.target, tgt);
        assert_eq!(rel.kind, RelationshipKind::Imports);
        assert_eq!(rel.span, Some(span));
    }

    #[test]
    fn graph_node_display() {
        assert_eq!(format!("{}", GraphNode::Crate(CrateId(1))), "Crate(CrateId(1))");
        assert_eq!(format!("{}", GraphNode::Module(ModuleId(2))), "Module(ModuleId(2))");
        assert_eq!(format!("{}", GraphNode::File(FileId(3))), "File(FileId(3))");
        assert_eq!(format!("{}", GraphNode::Symbol(SymbolId(4))), "Symbol(SymbolId(4))");
    }

    #[test]
    fn graph_node_equality() {
        assert_eq!(
            GraphNode::Symbol(SymbolId(1)),
            GraphNode::Symbol(SymbolId(1))
        );
        assert_ne!(
            GraphNode::Symbol(SymbolId(1)),
            GraphNode::Symbol(SymbolId(2))
        );
        assert_ne!(
            GraphNode::Symbol(SymbolId(1)),
            GraphNode::Crate(CrateId(1))
        );
    }

    #[test]
    fn graph_node_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(GraphNode::Crate(CrateId(1)));
        set.insert(GraphNode::Crate(CrateId(1)));
        set.insert(GraphNode::Module(ModuleId(1)));
        assert_eq!(set.len(), 2);
    }

    #[test]
    fn graph_edge_construction() {
        let edge = GraphEdge::new(RelationshipKind::Calls, 1.5);
        assert_eq!(edge.kind, RelationshipKind::Calls);
        assert!((edge.weight - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn arch_graph_add_nodes_and_edges() {
        let mut graph = ArchGraph::new();

        let n1 = graph.add_node(GraphNode::Crate(CrateId(0)));
        let n2 = graph.add_node(GraphNode::Module(ModuleId(0)));
        let n3 = graph.add_node(GraphNode::Symbol(SymbolId(0)));

        graph.add_edge(n1, n2, GraphEdge::new(RelationshipKind::Contains, 1.0));
        graph.add_edge(n2, n3, GraphEdge::new(RelationshipKind::Contains, 1.0));

        assert_eq!(graph.node_count(), 3);
        assert_eq!(graph.edge_count(), 2);
    }

    #[test]
    fn arch_graph_traverse_neighbors() {
        let gen = IdGenerator::new();
        let mut graph = ArchGraph::new();

        let crate_node = graph.add_node(GraphNode::Crate(gen.next_crate()));
        let mod_node = graph.add_node(GraphNode::Module(gen.next_module()));
        let sym1 = graph.add_node(GraphNode::Symbol(gen.next_symbol()));
        let sym2 = graph.add_node(GraphNode::Symbol(gen.next_symbol()));

        graph.add_edge(crate_node, mod_node, GraphEdge::new(RelationshipKind::Contains, 1.0));
        graph.add_edge(mod_node, sym1, GraphEdge::new(RelationshipKind::Contains, 1.0));
        graph.add_edge(mod_node, sym2, GraphEdge::new(RelationshipKind::Contains, 1.0));
        graph.add_edge(sym1, sym2, GraphEdge::new(RelationshipKind::Calls, 0.8));

        // mod_node has 2 outgoing Contains edges
        let neighbors: Vec<_> = graph.neighbors(mod_node).collect();
        assert_eq!(neighbors.len(), 2);

        // sym1 calls sym2
        let sym1_neighbors: Vec<_> = graph.neighbors(sym1).collect();
        assert_eq!(sym1_neighbors.len(), 1);
        assert_eq!(sym1_neighbors[0], sym2);
    }

    #[test]
    fn arch_graph_edge_weights() {
        let mut graph = ArchGraph::new();

        let n1 = graph.add_node(GraphNode::Symbol(SymbolId(0)));
        let n2 = graph.add_node(GraphNode::Symbol(SymbolId(1)));

        let edge_idx = graph.add_edge(
            n1,
            n2,
            GraphEdge::new(RelationshipKind::Calls, 2.5),
        );

        let edge = &graph[edge_idx];
        assert_eq!(edge.kind, RelationshipKind::Calls);
        assert!((edge.weight - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn serde_roundtrip_relationship_kind() {
        for kind in &[
            RelationshipKind::Contains,
            RelationshipKind::Imports,
            RelationshipKind::Calls,
            RelationshipKind::References,
            RelationshipKind::Implements,
            RelationshipKind::DependsOn,
        ] {
            let json = serde_json::to_string(kind).expect("serialize failed");
            let deser: RelationshipKind =
                serde_json::from_str(&json).expect("deserialize failed");
            assert_eq!(kind, &deser);
        }
    }

    #[test]
    fn serde_roundtrip_relationship() {
        let rel = Relationship::with_span(
            SymbolId(10),
            SymbolId(20),
            RelationshipKind::Implements,
            SourceSpan::new(FileId(5), 1, 0, 1, 30),
        );
        let json = serde_json::to_string(&rel).expect("serialize failed");
        let deser: Relationship = serde_json::from_str(&json).expect("deserialize failed");

        assert_eq!(rel.source, deser.source);
        assert_eq!(rel.target, deser.target);
        assert_eq!(rel.kind, deser.kind);
        assert_eq!(rel.span, deser.span);
    }

    #[test]
    fn serde_roundtrip_graph_node() {
        let nodes = vec![
            GraphNode::Crate(CrateId(1)),
            GraphNode::Module(ModuleId(2)),
            GraphNode::File(FileId(3)),
            GraphNode::Symbol(SymbolId(4)),
        ];
        for node in &nodes {
            let json = serde_json::to_string(node).expect("serialize failed");
            let deser: GraphNode = serde_json::from_str(&json).expect("deserialize failed");
            assert_eq!(node, &deser);
        }
    }

    #[test]
    fn serde_roundtrip_graph_edge() {
        let edge = GraphEdge::new(RelationshipKind::DependsOn, 0.75);
        let json = serde_json::to_string(&edge).expect("serialize failed");
        let deser: GraphEdge = serde_json::from_str(&json).expect("deserialize failed");

        assert_eq!(edge.kind, deser.kind);
        assert!((edge.weight - deser.weight).abs() < f32::EPSILON);
    }

    #[test]
    fn relationship_clone() {
        let rel = Relationship::new(SymbolId(1), SymbolId(2), RelationshipKind::Calls);
        let cloned = rel.clone();
        assert_eq!(rel.source, cloned.source);
        assert_eq!(rel.target, cloned.target);
        assert_eq!(rel.kind, cloned.kind);
        assert_eq!(rel.span, cloned.span);
    }

    #[test]
    fn graph_node_clone() {
        let node = GraphNode::Symbol(SymbolId(42));
        let cloned = node.clone();
        assert_eq!(node, cloned);
    }

    #[test]
    fn graph_edge_clone() {
        let edge = GraphEdge::new(RelationshipKind::References, 1.0);
        let cloned = edge.clone();
        assert_eq!(edge.kind, cloned.kind);
        assert!((edge.weight - cloned.weight).abs() < f32::EPSILON);
    }
}
