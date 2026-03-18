//! Symbol types: Symbol, SymbolKind, Visibility, SourceSpan, and SymbolAttributes.
//!
//! These types represent the individual code entities discovered during analysis:
//! functions, methods, structs, enums, traits, impl blocks, constants, statics,
//! and type aliases.

use serde::{Deserialize, Serialize};

use crate::id::{FileId, ModuleId, SymbolId};
use crate::traits::{Labeled, Spanned};

// ---------------------------------------------------------------------------
// SourceSpan
// ---------------------------------------------------------------------------

/// A contiguous region in a source file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    /// The file containing this span.
    pub file_id: FileId,
    /// Starting line (1-based).
    pub start_line: u32,
    /// Starting column (0-based byte offset within the line).
    pub start_col: u32,
    /// Ending line (1-based, inclusive).
    pub end_line: u32,
    /// Ending column (0-based byte offset within the line).
    pub end_col: u32,
}

impl SourceSpan {
    /// Create a new `SourceSpan`.
    pub fn new(
        file_id: FileId,
        start_line: u32,
        start_col: u32,
        end_line: u32,
        end_col: u32,
    ) -> Self {
        Self {
            file_id,
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

impl Spanned for SourceSpan {
    fn span(&self) -> &SourceSpan {
        self
    }

    fn file_id(&self) -> FileId {
        self.file_id
    }
}

// ---------------------------------------------------------------------------
// SymbolKind
// ---------------------------------------------------------------------------

/// The kind of symbol discovered during analysis.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    ImplBlock,
    Constant,
    Static,
    TypeAlias,
}

// ---------------------------------------------------------------------------
// Visibility
// ---------------------------------------------------------------------------

/// The visibility of a symbol in Rust source code.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Visibility {
    /// `pub`
    Public,
    /// `pub(crate)`
    Crate,
    /// `pub(in path)` or `pub(super)`
    Restricted,
    /// No visibility modifier (private to the containing module).
    Private,
}

// ---------------------------------------------------------------------------
// SymbolAttributes
// ---------------------------------------------------------------------------

/// Flags and metadata extracted during parsing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolAttributes {
    /// Whether the symbol is declared `async`.
    pub is_async: bool,
    /// Whether the symbol is declared `unsafe`.
    pub is_unsafe: bool,
    /// Whether the symbol is declared `extern`.
    pub is_extern: bool,
    /// Whether the symbol has a `#[test]` attribute.
    pub is_test: bool,
    /// Whether the body of the symbol contains an `unsafe { }` block.
    pub has_unsafe_block: bool,
    /// Documentation comment, if present.
    pub doc_comment: Option<String>,
    /// Raw attribute path strings extracted from the symbol.
    ///
    /// These are the paths of `#[...]` attributes applied to the symbol,
    /// e.g. `"tokio::main"`, `"test"`, `"get"`, `"handler"`, `"command"`.
    /// Used by the analysis engine for entrypoint detection and other
    /// heuristic passes.
    pub attribute_paths: Vec<String>,
}

impl SymbolAttributes {
    /// Create a default `SymbolAttributes` with all flags set to `false` and
    /// no doc comment.
    pub fn empty() -> Self {
        Self {
            is_async: false,
            is_unsafe: false,
            is_extern: false,
            is_test: false,
            has_unsafe_block: false,
            doc_comment: None,
            attribute_paths: Vec::new(),
        }
    }
}

impl Default for SymbolAttributes {
    fn default() -> Self {
        Self::empty()
    }
}

// ---------------------------------------------------------------------------
// Symbol
// ---------------------------------------------------------------------------

/// A code symbol discovered during analysis.
///
/// Represents a single identifiable entity in the source code: a function,
/// struct, enum, trait, impl block, constant, static, or type alias.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Symbol {
    /// Unique identifier for this symbol.
    pub id: SymbolId,
    /// Simple (unqualified) name of the symbol.
    pub name: String,
    /// What kind of symbol this is.
    pub kind: SymbolKind,
    /// The module in which this symbol is declared.
    pub module_id: ModuleId,
    /// The source file containing this symbol.
    pub file_id: FileId,
    /// The location of this symbol in source code.
    pub span: SourceSpan,
    /// The visibility of this symbol.
    pub visibility: Visibility,
    /// An optional human-readable signature (e.g. `"fn foo(x: i32) -> bool"`).
    pub signature: Option<String>,
    /// Parsed attributes and flags.
    pub attributes: SymbolAttributes,
}

impl Labeled for Symbol {
    fn label(&self) -> &str {
        &self.name
    }

    fn qualified_label(&self) -> String {
        // If a signature is available, use it as the qualified label since it
        // contains the full type information.  Otherwise, fall back to the name.
        match &self.signature {
            Some(sig) => sig.clone(),
            None => self.name.clone(),
        }
    }
}

impl Spanned for Symbol {
    fn span(&self) -> &SourceSpan {
        &self.span
    }

    fn file_id(&self) -> FileId {
        self.file_id
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdGenerator;

    /// Helper to create a minimal `Symbol` for testing.
    fn make_test_symbol(gen: &IdGenerator) -> Symbol {
        let sym_id = gen.next_symbol();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        Symbol {
            id: sym_id,
            name: "my_function".to_owned(),
            kind: SymbolKind::Function,
            module_id: mod_id,
            file_id,
            span: SourceSpan::new(file_id, 10, 0, 25, 1),
            visibility: Visibility::Public,
            signature: Some("fn my_function(x: i32) -> bool".to_owned()),
            attributes: SymbolAttributes {
                is_async: true,
                is_unsafe: false,
                is_extern: false,
                is_test: false,
                has_unsafe_block: false,
                doc_comment: Some("Does something useful.".to_owned()),
                attribute_paths: Vec::new(),
            },
        }
    }

    #[test]
    fn source_span_construction() {
        let fid = FileId(0);
        let span = SourceSpan::new(fid, 1, 0, 5, 20);
        assert_eq!(span.file_id, fid);
        assert_eq!(span.start_line, 1);
        assert_eq!(span.start_col, 0);
        assert_eq!(span.end_line, 5);
        assert_eq!(span.end_col, 20);
    }

    #[test]
    fn source_span_spanned_trait() {
        let fid = FileId(42);
        let span = SourceSpan::new(fid, 10, 4, 20, 8);

        // Spanned::span returns a reference to self
        let span_ref = Spanned::span(&span);
        assert_eq!(span_ref.start_line, 10);
        assert_eq!(span_ref.start_col, 4);
        assert_eq!(span_ref.end_line, 20);
        assert_eq!(span_ref.end_col, 8);

        // Spanned::file_id returns the file id
        assert_eq!(Spanned::file_id(&span), fid);
    }

    #[test]
    fn source_span_spanned_identity() {
        let fid = FileId(7);
        let span = SourceSpan::new(fid, 1, 0, 50, 0);

        // span() on a SourceSpan should return a reference to itself
        let returned = Spanned::span(&span);
        assert_eq!(returned, &span);
    }

    #[test]
    fn symbol_kind_variants() {
        let kinds = vec![
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Struct,
            SymbolKind::Enum,
            SymbolKind::Trait,
            SymbolKind::ImplBlock,
            SymbolKind::Constant,
            SymbolKind::Static,
            SymbolKind::TypeAlias,
        ];
        // All variants should be distinct
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }

    #[test]
    fn visibility_variants() {
        assert_ne!(Visibility::Public, Visibility::Private);
        assert_ne!(Visibility::Crate, Visibility::Restricted);
        assert_eq!(Visibility::Public, Visibility::Public);
    }

    #[test]
    fn symbol_attributes_empty() {
        let attrs = SymbolAttributes::empty();
        assert!(!attrs.is_async);
        assert!(!attrs.is_unsafe);
        assert!(!attrs.is_extern);
        assert!(!attrs.is_test);
        assert!(!attrs.has_unsafe_block);
        assert!(attrs.doc_comment.is_none());
    }

    #[test]
    fn symbol_attributes_default_matches_empty() {
        let default = SymbolAttributes::default();
        let empty = SymbolAttributes::empty();
        assert_eq!(default, empty);
    }

    #[test]
    fn symbol_construction() {
        let gen = IdGenerator::new();
        let sym = make_test_symbol(&gen);

        assert_eq!(sym.name, "my_function");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert_eq!(sym.visibility, Visibility::Public);
        assert!(sym.signature.is_some());
        assert!(sym.attributes.is_async);
        assert!(!sym.attributes.is_unsafe);
    }

    #[test]
    fn symbol_labeled_trait() {
        let gen = IdGenerator::new();
        let sym = make_test_symbol(&gen);

        assert_eq!(sym.label(), "my_function");
        assert_eq!(
            sym.qualified_label(),
            "fn my_function(x: i32) -> bool"
        );
    }

    #[test]
    fn symbol_labeled_trait_without_signature() {
        let gen = IdGenerator::new();
        let mut sym = make_test_symbol(&gen);
        sym.signature = None;

        assert_eq!(sym.label(), "my_function");
        assert_eq!(sym.qualified_label(), "my_function");
    }

    #[test]
    fn symbol_spanned_trait() {
        let gen = IdGenerator::new();
        let sym = make_test_symbol(&gen);

        let span = Spanned::span(&sym);
        assert_eq!(span.start_line, 10);
        assert_eq!(span.end_line, 25);

        assert_eq!(Spanned::file_id(&sym), sym.file_id);
    }

    #[test]
    fn symbol_clone() {
        let gen = IdGenerator::new();
        let sym = make_test_symbol(&gen);
        let cloned = sym.clone();

        assert_eq!(sym.id, cloned.id);
        assert_eq!(sym.name, cloned.name);
        assert_eq!(sym.kind, cloned.kind);
    }

    #[test]
    fn serde_roundtrip_source_span() {
        let span = SourceSpan::new(FileId(7), 1, 4, 10, 30);
        let json = serde_json::to_string(&span).expect("serialize failed");
        let deser: SourceSpan = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(span, deser);
    }

    #[test]
    fn serde_roundtrip_symbol_kind() {
        let kind = SymbolKind::ImplBlock;
        let json = serde_json::to_string(&kind).expect("serialize failed");
        let deser: SymbolKind = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(kind, deser);
    }

    #[test]
    fn serde_roundtrip_visibility() {
        for vis in &[
            Visibility::Public,
            Visibility::Crate,
            Visibility::Restricted,
            Visibility::Private,
        ] {
            let json = serde_json::to_string(vis).expect("serialize failed");
            let deser: Visibility = serde_json::from_str(&json).expect("deserialize failed");
            assert_eq!(vis, &deser);
        }
    }

    #[test]
    fn serde_roundtrip_symbol_attributes() {
        let attrs = SymbolAttributes {
            is_async: true,
            is_unsafe: true,
            is_extern: false,
            is_test: true,
            has_unsafe_block: true,
            doc_comment: Some("A doc comment.".to_owned()),
            attribute_paths: Vec::new(),
        };
        let json = serde_json::to_string(&attrs).expect("serialize failed");
        let deser: SymbolAttributes =
            serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(attrs, deser);
    }

    #[test]
    fn serde_roundtrip_symbol() {
        let gen = IdGenerator::new();
        let sym = make_test_symbol(&gen);
        let json = serde_json::to_string(&sym).expect("serialize failed");
        let deser: Symbol = serde_json::from_str(&json).expect("deserialize failed");

        assert_eq!(sym.id, deser.id);
        assert_eq!(sym.name, deser.name);
        assert_eq!(sym.kind, deser.kind);
        assert_eq!(sym.module_id, deser.module_id);
        assert_eq!(sym.file_id, deser.file_id);
        assert_eq!(sym.span, deser.span);
        assert_eq!(sym.visibility, deser.visibility);
        assert_eq!(sym.signature, deser.signature);
        assert_eq!(sym.attributes, deser.attributes);
    }
}
