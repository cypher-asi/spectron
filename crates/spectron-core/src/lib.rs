//! spectron-core: domain types and shared abstractions for Spectron.

pub mod analysis;
pub mod error;
pub mod graph;
pub mod id;
pub mod metrics;
pub mod project;
pub mod security;
pub mod symbol;
pub mod traits;

// Re-export identity types at the crate root for convenience.
pub use id::{CrateId, FileId, IdGenerator, ModuleId, SymbolId};

// Re-export project hierarchy types at the crate root for convenience.
pub use project::{CrateInfo, CrateType, ModuleInfo, ModulePath, ProjectInfo};

// Re-export symbol types at the crate root for convenience.
pub use symbol::{SourceSpan, Symbol, SymbolAttributes, SymbolKind, Visibility};

// Re-export relationship and graph types at the crate root for convenience.
pub use graph::{ArchGraph, GraphEdge, GraphNode, Relationship, RelationshipKind};

// Re-export shared traits at the crate root for convenience.
pub use traits::{Labeled, Spanned};

// Re-export metric types at the crate root for convenience.
pub use metrics::{ModuleMetrics, SymbolMetrics};

// Re-export security types at the crate root for convenience.
pub use security::{SecurityIndicator, SecurityReport};

// Re-export analysis output types at the crate root for convenience.
pub use analysis::{AnalysisResult, FileInfo, ParseError};

// Re-export error types at the crate root for convenience.
pub use error::SpectronError;
