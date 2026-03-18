//! Analysis output types: FileInfo, ParseError, and AnalysisResult.
//!
//! These types represent the top-level output of a complete analysis pass,
//! aggregating all discovered project structure, symbols, relationships,
//! metrics, and security indicators.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::graph::{ArchGraph, Relationship};
use crate::id::{CrateId, FileId, ModuleId, SymbolId};
use crate::metrics::{ModuleMetrics, SymbolMetrics};
use crate::project::{CrateInfo, ModuleInfo, ProjectInfo};
use crate::security::SecurityReport;
use crate::symbol::{SourceSpan, Symbol};

// ---------------------------------------------------------------------------
// FileInfo
// ---------------------------------------------------------------------------

/// Metadata about a source file in the analyzed project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileInfo {
    /// Unique identifier for this file.
    pub id: FileId,
    /// Path to the source file.
    pub path: PathBuf,
    /// Content hash of the file (for change detection).
    pub hash: String,
    /// Total number of lines in the file.
    pub line_count: u32,
}

impl FileInfo {
    /// Create a new `FileInfo`.
    pub fn new(
        id: FileId,
        path: impl Into<PathBuf>,
        hash: impl Into<String>,
        line_count: u32,
    ) -> Self {
        Self {
            id,
            path: path.into(),
            hash: hash.into(),
            line_count,
        }
    }
}

// ---------------------------------------------------------------------------
// ParseError
// ---------------------------------------------------------------------------

/// An error encountered while parsing a source file.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ParseError {
    /// Path to the file that failed to parse.
    pub file_path: PathBuf,
    /// Human-readable error message.
    pub message: String,
    /// Optional source span where the error occurred.
    pub span: Option<SourceSpan>,
}

impl ParseError {
    /// Create a new `ParseError` without a span.
    pub fn new(file_path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self {
            file_path: file_path.into(),
            message: message.into(),
            span: None,
        }
    }

    /// Create a new `ParseError` with a source span.
    pub fn with_span(
        file_path: impl Into<PathBuf>,
        message: impl Into<String>,
        span: SourceSpan,
    ) -> Self {
        Self {
            file_path: file_path.into(),
            message: message.into(),
            span: Some(span),
        }
    }
}

// ---------------------------------------------------------------------------
// AnalysisResult
// ---------------------------------------------------------------------------

/// The top-level result of a complete analysis pass.
///
/// This struct aggregates all data produced by the analysis pipeline: project
/// structure, symbols, relationships, the architecture graph, metrics, security
/// indicators, entry points, and any parse errors encountered.
pub struct AnalysisResult {
    /// Root project metadata.
    pub project: ProjectInfo,
    /// All crates, indexed by their ID.
    pub crates: HashMap<CrateId, CrateInfo>,
    /// All modules, indexed by their ID.
    pub modules: HashMap<ModuleId, ModuleInfo>,
    /// All symbols, indexed by their ID.
    pub symbols: HashMap<SymbolId, Symbol>,
    /// All source files, indexed by their ID.
    pub files: HashMap<FileId, FileInfo>,
    /// All relationships discovered during analysis.
    pub relationships: Vec<Relationship>,
    /// The architecture graph connecting all entities.
    pub graph: ArchGraph,
    /// Per-symbol metrics (complexity, line count, etc.).
    pub symbol_metrics: HashMap<SymbolId, SymbolMetrics>,
    /// Per-module metrics (fan-in, fan-out, etc.).
    pub module_metrics: HashMap<ModuleId, ModuleMetrics>,
    /// Security report with all detected indicators.
    pub security_report: SecurityReport,
    /// Symbols identified as entry points (e.g. `main`, test functions).
    pub entrypoints: Vec<SymbolId>,
    /// Errors encountered during parsing.
    pub parse_errors: Vec<ParseError>,
}

impl AnalysisResult {
    /// Create a new `AnalysisResult` with the given project info and empty collections.
    pub fn new(project: ProjectInfo) -> Self {
        Self {
            project,
            crates: HashMap::new(),
            modules: HashMap::new(),
            symbols: HashMap::new(),
            files: HashMap::new(),
            relationships: Vec::new(),
            graph: ArchGraph::new(),
            symbol_metrics: HashMap::new(),
            module_metrics: HashMap::new(),
            security_report: SecurityReport::new(),
            entrypoints: Vec::new(),
            parse_errors: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::FileId;
    use crate::IdGenerator;

    #[test]
    fn file_info_construction() {
        let info = FileInfo::new(FileId(0), "src/main.rs", "abc123", 150);
        assert_eq!(info.id, FileId(0));
        assert_eq!(info.path, PathBuf::from("src/main.rs"));
        assert_eq!(info.hash, "abc123");
        assert_eq!(info.line_count, 150);
    }

    #[test]
    fn file_info_clone() {
        let info = FileInfo::new(FileId(1), "src/lib.rs", "def456", 200);
        let cloned = info.clone();
        assert_eq!(info.id, cloned.id);
        assert_eq!(info.path, cloned.path);
        assert_eq!(info.hash, cloned.hash);
        assert_eq!(info.line_count, cloned.line_count);
    }

    #[test]
    fn parse_error_without_span() {
        let err = ParseError::new("src/bad.rs", "unexpected token");
        assert_eq!(err.file_path, PathBuf::from("src/bad.rs"));
        assert_eq!(err.message, "unexpected token");
        assert!(err.span.is_none());
    }

    #[test]
    fn parse_error_with_span() {
        let span = SourceSpan::new(FileId(0), 10, 5, 10, 20);
        let err = ParseError::with_span("src/bad.rs", "unexpected token", span.clone());
        assert_eq!(err.file_path, PathBuf::from("src/bad.rs"));
        assert_eq!(err.message, "unexpected token");
        assert_eq!(err.span, Some(span));
    }

    #[test]
    fn parse_error_clone() {
        let err = ParseError::new("src/test.rs", "syntax error");
        let cloned = err.clone();
        assert_eq!(err.file_path, cloned.file_path);
        assert_eq!(err.message, cloned.message);
        assert_eq!(err.span, cloned.span);
    }

    #[test]
    fn analysis_result_new_has_empty_collections() {
        let project = ProjectInfo::new("test-project", "/tmp/test", false);
        let result = AnalysisResult::new(project);

        assert_eq!(result.project.name, "test-project");
        assert!(result.crates.is_empty());
        assert!(result.modules.is_empty());
        assert!(result.symbols.is_empty());
        assert!(result.files.is_empty());
        assert!(result.relationships.is_empty());
        assert_eq!(result.graph.node_count(), 0);
        assert_eq!(result.graph.edge_count(), 0);
        assert!(result.symbol_metrics.is_empty());
        assert!(result.module_metrics.is_empty());
        assert!(result.security_report.indicators.is_empty());
        assert!(result.entrypoints.is_empty());
        assert!(result.parse_errors.is_empty());
    }

    #[test]
    fn analysis_result_populate_collections() {
        let gen = IdGenerator::new();
        let project = ProjectInfo::new("proj", "/tmp/proj", false);
        let mut result = AnalysisResult::new(project);

        // Add a file
        let file_id = gen.next_file();
        result
            .files
            .insert(file_id, FileInfo::new(file_id, "src/lib.rs", "hash1", 100));
        assert_eq!(result.files.len(), 1);

        // Add a parse error
        result
            .parse_errors
            .push(ParseError::new("src/broken.rs", "bad syntax"));
        assert_eq!(result.parse_errors.len(), 1);

        // Add an entrypoint
        let sym_id = gen.next_symbol();
        result.entrypoints.push(sym_id);
        assert_eq!(result.entrypoints.len(), 1);
    }

    #[test]
    fn serde_roundtrip_file_info() {
        let info = FileInfo::new(FileId(7), "src/foo.rs", "deadbeef", 42);
        let json = serde_json::to_string(&info).expect("serialize failed");
        let deser: FileInfo = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(info.id, deser.id);
        assert_eq!(info.path, deser.path);
        assert_eq!(info.hash, deser.hash);
        assert_eq!(info.line_count, deser.line_count);
    }

    #[test]
    fn serde_roundtrip_parse_error() {
        let span = SourceSpan::new(FileId(0), 5, 0, 5, 15);
        let err = ParseError::with_span("src/mod.rs", "missing semicolon", span);
        let json = serde_json::to_string(&err).expect("serialize failed");
        let deser: ParseError = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(err.file_path, deser.file_path);
        assert_eq!(err.message, deser.message);
        assert_eq!(err.span, deser.span);
    }
}
