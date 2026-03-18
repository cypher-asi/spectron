//! spectron-graph: graph construction (structure, call, CFG, dataflow).
//!
//! This crate consumes parsed symbols and relationships and constructs
//! typed graph representations including structure graphs, call graphs,
//! control flow graphs, and data flow information.

pub mod algorithms;
pub mod builder;
pub mod cfg;

// Re-export algorithm utilities at the crate root for convenience.
pub use algorithms::{
    components, descendants, find_paths, reachable_subgraph, DataFlowInfo,
};

// Re-export CFG types at the crate root for convenience.
pub use cfg::{
    build_cfg_from_stmts, build_cfgs_from_source, CfgEdge, CfgNode, ControlFlowGraph,
};

// Re-export builder types and entry point at the crate root for convenience.
pub use builder::{build_graphs, CallGraphData, GraphIndex, GraphSet, UNRESOLVED_MODULE_ID};
