//! spectron-parser: Rust semantic parsing and symbol extraction.
//!
//! This crate consumes the file list and module tree from `spectron-loader` and
//! produces [`Symbol`] entries for every detectable entity, [`Relationship`]
//! entries for every detectable relationship, and [`ParseError`] entries for
//! files that fail to parse.

pub mod resolve;
pub mod visitor;

use std::collections::HashMap;
use std::path::PathBuf;

use rayon::prelude::*;
use syn::visit::Visit;

use spectron_core::{
    FileId, IdGenerator, ModuleId, ParseError, Relationship, SourceSpan, Symbol,
};
use spectron_loader::LoadResult;

use crate::resolve::{resolve_imports, resolve_relationships, SymbolTable};
use crate::visitor::{ExtractedRelationship, ExtractedSymbol, SymbolVisitor};

/// The result of parsing an entire project.
///
/// Contains all symbols and relationships extracted from the source files,
/// along with any errors encountered during parsing.
#[derive(Debug)]
pub struct ParseResult {
    /// All symbols extracted from the project source files.
    pub symbols: Vec<Symbol>,
    /// All relationships discovered during parsing.
    pub relationships: Vec<Relationship>,
    /// Errors encountered while parsing individual files.
    pub errors: Vec<ParseError>,
}

impl ParseResult {
    /// Create an empty `ParseResult`.
    pub fn new() -> Self {
        Self {
            symbols: Vec::new(),
            relationships: Vec::new(),
            errors: Vec::new(),
        }
    }
}

impl Default for ParseResult {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// File-level parse result (collected per file in parallel)
// ---------------------------------------------------------------------------

/// The result of parsing a single file. Collected in parallel, then merged.
struct SingleFileResult {
    file_id: FileId,
    module_id: ModuleId,
    symbols: Vec<ExtractedSymbol>,
    relationships: Vec<ExtractedRelationship>,
}

/// An error from parsing a single file.
struct SingleFileError {
    file_path: PathBuf,
    file_id: FileId,
    message: String,
    /// Optional line/column for the parse error.
    span: Option<(u32, u32)>,
}

/// The outcome of attempting to parse one file.
enum FileOutcome {
    /// File was parsed successfully.
    Ok(SingleFileResult),
    /// File could not be read or parsed.
    Err(SingleFileError),
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse all source files and extract symbols + relationships.
///
/// This is the main entry point for the parser crate. It:
///
/// 1. Builds a mapping from file paths to `(FileId, ModuleId)` using the
///    module tree from the `LoadResult`.
/// 2. Parses all source files in parallel using `rayon::par_iter()`.
/// 3. For each file, reads the contents, parses with `syn::parse_file()`,
///    and walks the AST with [`SymbolVisitor`].
/// 4. On parse failure, records a [`ParseError`] and continues.
/// 5. After all files are parsed, runs name resolution to convert
///    name-based references into `SymbolId`-based [`Relationship`]s.
/// 6. Returns a [`ParseResult`] with all symbols, relationships, and errors.
pub fn parse_project(load_result: &LoadResult) -> ParseResult {
    let id_gen = IdGenerator::new();

    // Step 1: Build the file path -> (FileId, ModuleId) mapping.
    let file_module_map = build_file_module_map(load_result);

    // Step 2: Parse all files in parallel.
    let outcomes: Vec<FileOutcome> = load_result
        .files
        .par_iter()
        .map(|file_info| {
            let module_id = file_module_map
                .get(&file_info.path)
                .copied()
                .unwrap_or(ModuleId(0));

            parse_single_file(&file_info.path, file_info.id, module_id)
        })
        .collect();

    // Step 3: Separate successes from errors and flatten results.
    let mut all_extracted_symbols: Vec<(FileId, ModuleId, ExtractedSymbol)> = Vec::new();
    let mut all_extracted_rels: Vec<(FileId, ModuleId, ExtractedRelationship)> = Vec::new();
    let mut errors: Vec<ParseError> = Vec::new();

    for outcome in outcomes {
        match outcome {
            FileOutcome::Ok(result) => {
                for sym in result.symbols {
                    all_extracted_symbols.push((result.file_id, result.module_id, sym));
                }
                for rel in result.relationships {
                    all_extracted_rels.push((result.file_id, result.module_id, rel));
                }
            }
            FileOutcome::Err(err) => {
                let parse_error = if let Some((line, col)) = err.span {
                    ParseError::with_span(
                        err.file_path,
                        err.message,
                        SourceSpan::new(err.file_id, line, col, line, col),
                    )
                } else {
                    ParseError::new(err.file_path, err.message)
                };
                errors.push(parse_error);
            }
        }
    }

    // Step 4: Convert extracted symbols to fully formed Symbol structs.
    let symbols: Vec<Symbol> = all_extracted_symbols
        .into_iter()
        .map(|(file_id, module_id, ext)| Symbol {
            id: id_gen.next_symbol(),
            name: ext.name,
            kind: ext.kind,
            module_id,
            file_id,
            span: SourceSpan::new(
                file_id,
                ext.span_start_line,
                ext.span_start_col,
                ext.span_end_line,
                ext.span_end_col,
            ),
            visibility: ext.visibility,
            signature: ext.signature,
            attributes: ext.attributes,
        })
        .collect();

    // Step 5: Run name resolution.
    let module_parents = build_module_parents(load_result);
    let mut table = SymbolTable::build(&symbols, &module_parents);

    // First pass: resolve imports so they are available for call resolution.
    let import_rels: Vec<_> = all_extracted_rels
        .iter()
        .filter(|(_, _, rel)| rel.kind == spectron_core::RelationshipKind::Imports)
        .cloned()
        .collect();
    resolve_imports(&mut table, &import_rels);

    // Second pass: resolve all relationships to SymbolId-based Relationships.
    let relationships = resolve_relationships(&table, &symbols, &all_extracted_rels);

    ParseResult {
        symbols,
        relationships,
        errors,
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Build a mapping from canonical file path -> ModuleId.
///
/// Iterates over all modules in the load result and maps their `file_path`
/// (if present) to their `ModuleId`. This allows the parser to assign the
/// correct module ID to each file it processes.
fn build_file_module_map(load_result: &LoadResult) -> HashMap<PathBuf, ModuleId> {
    let mut map = HashMap::new();

    for module in &load_result.modules {
        if let Some(ref file_path) = module.file_path {
            // If multiple modules map to the same file (e.g. due to dual
            // targets), the first one wins. This is acceptable because
            // symbols extracted from the same file will have the same content
            // regardless of which module ID is assigned.
            map.entry(file_path.clone()).or_insert(module.id);
        }
    }

    map
}

/// Build a mapping from child ModuleId -> parent ModuleId for name resolution.
fn build_module_parents(load_result: &LoadResult) -> HashMap<ModuleId, ModuleId> {
    let mut parents = HashMap::new();

    for module in &load_result.modules {
        if let Some(parent_id) = module.parent {
            parents.insert(module.id, parent_id);
        }
    }

    parents
}

/// Parse a single source file, returning either extracted symbols/relationships
/// or a parse error.
fn parse_single_file(
    file_path: &std::path::Path,
    file_id: FileId,
    module_id: ModuleId,
) -> FileOutcome {
    // Read the file contents.
    let content = match std::fs::read_to_string(file_path) {
        Ok(content) => content,
        Err(e) => {
            return FileOutcome::Err(SingleFileError {
                file_path: file_path.to_path_buf(),
                file_id,
                message: format!("failed to read file: {}", e),
                span: None,
            });
        }
    };

    // Log a warning for very large files (>10k lines).
    let line_count = content.lines().count();
    if line_count > 10_000 {
        tracing::warn!(
            path = %file_path.display(),
            lines = line_count,
            "parsing very large file (>10k lines)"
        );
    }

    // Parse with syn.
    let ast = match syn::parse_file(&content) {
        Ok(ast) => ast,
        Err(e) => {
            let span_loc = e.span().start();
            return FileOutcome::Err(SingleFileError {
                file_path: file_path.to_path_buf(),
                file_id,
                message: format!("{}", e),
                span: Some((span_loc.line as u32, span_loc.column as u32)),
            });
        }
    };

    // Walk the AST with our visitor.
    let mut visitor = SymbolVisitor::new(file_id, module_id);
    visitor.visit_file(&ast);
    let (symbols, relationships) = visitor.into_results();

    FileOutcome::Ok(SingleFileResult {
        file_id,
        module_id,
        symbols,
        relationships,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_result_default_is_empty() {
        let result = ParseResult::default();
        assert!(result.symbols.is_empty());
        assert!(result.relationships.is_empty());
        assert!(result.errors.is_empty());
    }

    #[test]
    fn parse_result_new_is_empty() {
        let result = ParseResult::new();
        assert!(result.symbols.is_empty());
        assert!(result.relationships.is_empty());
        assert!(result.errors.is_empty());
    }
}
