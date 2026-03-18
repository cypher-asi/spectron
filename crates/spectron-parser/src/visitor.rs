//! AST visitor for extracting symbols and relationships from Rust source files.
//!
//! Implements `syn::visit::Visit` to walk parsed ASTs and collect
//! [`Symbol`] entries for every detectable entity: functions, methods,
//! structs, enums, traits, impl blocks, constants, statics, and type aliases.
//!
//! Also extracts [`ExtractedRelationship`] entries for imports, trait
//! implementations, function calls, method calls, and path references.

use std::sync::Mutex;

use quote::ToTokens;
use syn::visit::Visit;
use syn::spanned::Spanned as SynSpanned;

use spectron_core::{
    FileId, IdGenerator, ModuleId, RelationshipKind, SourceSpan, Symbol, SymbolAttributes,
    SymbolKind, Visibility,
};

// ---------------------------------------------------------------------------
// Extracted symbol (intermediate representation)
// ---------------------------------------------------------------------------

/// An intermediate representation of a symbol extracted by the visitor,
/// before final `Symbol` construction. This avoids carrying around the
/// `IdGenerator` inside the visitor callbacks.
#[derive(Clone, Debug)]
pub struct ExtractedSymbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: Visibility,
    pub span_start_line: u32,
    pub span_start_col: u32,
    pub span_end_line: u32,
    pub span_end_col: u32,
    pub signature: Option<String>,
    pub attributes: SymbolAttributes,
}

// ---------------------------------------------------------------------------
// Thread-safe accumulator
// ---------------------------------------------------------------------------

/// A thread-safe accumulator for symbols extracted across multiple files.
///
/// Wraps a `Mutex<Vec<(FileId, ModuleId, ExtractedSymbol)>>` so that
/// visitors running on different threads (via rayon) can safely push
/// results without external synchronization.
#[derive(Debug)]
pub struct SymbolAccumulator {
    inner: Mutex<Vec<(FileId, ModuleId, ExtractedSymbol)>>,
}

impl SymbolAccumulator {
    /// Create a new empty accumulator.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Push an extracted symbol into the accumulator.
    pub fn push(&self, file_id: FileId, module_id: ModuleId, sym: ExtractedSymbol) {
        let mut guard = self.inner.lock().expect("accumulator lock poisoned");
        guard.push((file_id, module_id, sym));
    }

    /// Drain the accumulator and convert all entries into fully formed
    /// [`Symbol`] values using the given `IdGenerator`.
    pub fn into_symbols(self, id_gen: &IdGenerator) -> Vec<Symbol> {
        let entries = self.inner.into_inner().expect("accumulator lock poisoned");
        entries
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
            .collect()
    }

    /// Drain the accumulator into raw extracted entries.
    pub fn into_entries(self) -> Vec<(FileId, ModuleId, ExtractedSymbol)> {
        self.inner.into_inner().expect("accumulator lock poisoned")
    }
}

impl Default for SymbolAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Extracted relationship (intermediate representation)
// ---------------------------------------------------------------------------

/// An intermediate representation of a relationship extracted by the visitor,
/// before name resolution. The target is stored as a string name that will be
/// resolved to a `SymbolId` in a later phase.
#[derive(Clone, Debug)]
pub struct ExtractedRelationship {
    /// Name of the source symbol (current function, method, or module context).
    pub source_name: String,
    /// Name of the target symbol (to be resolved to a `SymbolId` later).
    pub target_name: String,
    /// The full path of the target, if available (e.g. `std::fs::read`).
    pub target_path: Option<String>,
    /// What kind of relationship this is.
    pub kind: RelationshipKind,
    /// Span start line.
    pub span_start_line: u32,
    /// Span start column.
    pub span_start_col: u32,
    /// Span end line.
    pub span_end_line: u32,
    /// Span end column.
    pub span_end_col: u32,
}

// ---------------------------------------------------------------------------
// Thread-safe relationship accumulator
// ---------------------------------------------------------------------------

/// A thread-safe accumulator for relationships extracted across multiple files.
///
/// Wraps a `Mutex<Vec<(FileId, ModuleId, ExtractedRelationship)>>` so that
/// visitors running on different threads (via rayon) can safely push
/// results without external synchronization.
#[derive(Debug)]
pub struct RelationshipAccumulator {
    inner: Mutex<Vec<(FileId, ModuleId, ExtractedRelationship)>>,
}

impl RelationshipAccumulator {
    /// Create a new empty accumulator.
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Vec::new()),
        }
    }

    /// Push an extracted relationship into the accumulator.
    pub fn push(
        &self,
        file_id: FileId,
        module_id: ModuleId,
        rel: ExtractedRelationship,
    ) {
        let mut guard = self.inner.lock().expect("relationship accumulator lock poisoned");
        guard.push((file_id, module_id, rel));
    }

    /// Drain the accumulator into raw extracted entries.
    pub fn into_entries(self) -> Vec<(FileId, ModuleId, ExtractedRelationship)> {
        self.inner
            .into_inner()
            .expect("relationship accumulator lock poisoned")
    }
}

impl Default for RelationshipAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// SymbolVisitor
// ---------------------------------------------------------------------------

/// AST visitor that extracts symbols and relationships from a parsed Rust
/// source file.
///
/// Create one per file, walk the AST with [`syn::visit::visit_file`],
/// then collect the results from `symbols` and `relationships`.
pub struct SymbolVisitor {
    /// The file ID for the current source file.
    file_id: FileId,
    /// The module ID for the current source file.
    module_id: ModuleId,
    /// Accumulated symbols for this file.
    symbols: Vec<ExtractedSymbol>,
    /// Accumulated relationships for this file.
    relationships: Vec<ExtractedRelationship>,
    /// Name of the current impl block target type, if inside one.
    current_impl_type: Option<String>,
    /// Name of the current function or method, if inside one.
    /// Used as the source context for call/reference relationships.
    current_function: Option<String>,
}

impl SymbolVisitor {
    /// Create a new visitor for a specific file and module.
    pub fn new(file_id: FileId, module_id: ModuleId) -> Self {
        Self {
            file_id,
            module_id,
            symbols: Vec::new(),
            relationships: Vec::new(),
            current_impl_type: None,
            current_function: None,
        }
    }

    /// Return the file ID this visitor was created for.
    pub fn file_id(&self) -> FileId {
        self.file_id
    }

    /// Return the module ID this visitor was created for.
    pub fn module_id(&self) -> ModuleId {
        self.module_id
    }

    /// Consume the visitor and return the extracted symbols.
    pub fn into_symbols(self) -> Vec<ExtractedSymbol> {
        self.symbols
    }

    /// Consume the visitor and return both extracted symbols and relationships.
    pub fn into_results(self) -> (Vec<ExtractedSymbol>, Vec<ExtractedRelationship>) {
        (self.symbols, self.relationships)
    }

    /// Borrow the extracted symbols.
    pub fn symbols(&self) -> &[ExtractedSymbol] {
        &self.symbols
    }

    /// Borrow the extracted relationships.
    pub fn relationships(&self) -> &[ExtractedRelationship] {
        &self.relationships
    }

    /// Push a symbol and record results into an accumulator.
    fn record(&mut self, sym: ExtractedSymbol) {
        self.symbols.push(sym);
    }

    /// Record an extracted relationship.
    fn record_relationship(&mut self, rel: ExtractedRelationship) {
        self.relationships.push(rel);
    }

    /// Get the current context name for relationship sources.
    ///
    /// Returns the current function/method name if inside one,
    /// otherwise returns "<module>" as a placeholder.
    fn current_context_name(&self) -> String {
        if let Some(ref func) = self.current_function {
            func.clone()
        } else {
            "<module>".to_string()
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Check whether a list of attributes contains `#[test]` or `#[tokio::test]`.
fn has_test_attribute(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if let syn::Meta::Path(ref path) = attr.meta {
            let segments: Vec<String> = path
                .segments
                .iter()
                .map(|seg| seg.ident.to_string())
                .collect();
            if segments == ["test"] {
                return true;
            }
            if segments == ["tokio", "test"] {
                return true;
            }
        }
        false
    })
}

/// Extract documentation comments from attributes.
///
/// This handles both `/// doc comment` style (which syn normalizes to
/// `#[doc = "..."]`) and explicit `#[doc = "..."]` attributes. Lines are
/// joined with newlines and leading/trailing whitespace is trimmed on the
/// final result.
fn extract_doc_comment(attrs: &[syn::Attribute]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("doc") {
            continue;
        }
        // syn represents `/// text` as `#[doc = " text"]`
        if let syn::Meta::NameValue(ref nv) = attr.meta {
            if let syn::Expr::Lit(ref expr_lit) = nv.value {
                if let syn::Lit::Str(ref lit_str) = expr_lit.lit {
                    lines.push(lit_str.value());
                }
            }
        }
    }
    if lines.is_empty() {
        None
    } else {
        let joined = lines.join("\n").trim().to_string();
        if joined.is_empty() {
            None
        } else {
            Some(joined)
        }
    }
}

/// Walk a block looking for `unsafe { ... }` expressions.
///
/// Returns `true` if any `ExprUnsafe` node is found anywhere in the block.
fn block_contains_unsafe(block: &syn::Block) -> bool {
    struct UnsafeDetector {
        found: bool,
    }
    impl<'ast> Visit<'ast> for UnsafeDetector {
        fn visit_expr_unsafe(&mut self, _node: &'ast syn::ExprUnsafe) {
            self.found = true;
            // No need to continue walking once we found one.
        }
    }
    let mut detector = UnsafeDetector { found: false };
    syn::visit::visit_block(&mut detector, block);
    detector.found
}

/// Convert a `syn::Visibility` into our domain `Visibility`.
fn convert_visibility(vis: &syn::Visibility) -> Visibility {
    match vis {
        syn::Visibility::Public(_) => Visibility::Public,
        syn::Visibility::Restricted(r) => {
            let path_str = r.path.to_token_stream().to_string();
            if path_str == "crate" {
                Visibility::Crate
            } else {
                Visibility::Restricted
            }
        }
        syn::Visibility::Inherited => Visibility::Private,
    }
}

/// Extract line/column from a `proc_macro2::Span`.
///
/// `syn` spans in non-proc-macro context (i.e. `syn::parse_str` or
/// `syn::parse_file`) report 1-based lines and 0-based columns.
/// We store them as-is in `SourceSpan`.
fn extract_span(spanned: &impl SynSpanned) -> (u32, u32, u32, u32) {
    let span = spanned.span();
    let start = span.start();
    let end = span.end();
    (
        start.line as u32,
        start.column as u32,
        end.line as u32,
        end.column as u32,
    )
}

/// Reconstruct a human-readable signature for a function item.
fn fn_signature(item: &syn::ItemFn) -> String {
    let mut sig = String::new();

    if item.sig.constness.is_some() {
        sig.push_str("const ");
    }
    if item.sig.asyncness.is_some() {
        sig.push_str("async ");
    }
    if item.sig.unsafety.is_some() {
        sig.push_str("unsafe ");
    }
    if let Some(ref abi) = item.sig.abi {
        sig.push_str("extern ");
        if let Some(ref name) = abi.name {
            sig.push_str(&name.value());
            sig.push(' ');
        }
    }

    sig.push_str("fn ");
    sig.push_str(&item.sig.ident.to_string());

    sig.push('(');
    let params: Vec<String> = item
        .sig
        .inputs
        .iter()
        .map(|arg| arg.to_token_stream().to_string())
        .collect();
    sig.push_str(&params.join(", "));
    sig.push(')');

    if let syn::ReturnType::Type(_, ref ty) = item.sig.output {
        sig.push_str(" -> ");
        sig.push_str(&ty.to_token_stream().to_string());
    }

    sig
}

/// Reconstruct a human-readable signature for a method (impl item fn).
fn method_signature(item: &syn::ImplItemFn) -> String {
    let mut sig = String::new();

    if item.sig.constness.is_some() {
        sig.push_str("const ");
    }
    if item.sig.asyncness.is_some() {
        sig.push_str("async ");
    }
    if item.sig.unsafety.is_some() {
        sig.push_str("unsafe ");
    }
    if let Some(ref abi) = item.sig.abi {
        sig.push_str("extern ");
        if let Some(ref name) = abi.name {
            sig.push_str(&name.value());
            sig.push(' ');
        }
    }

    sig.push_str("fn ");
    sig.push_str(&item.sig.ident.to_string());

    sig.push('(');
    let params: Vec<String> = item
        .sig
        .inputs
        .iter()
        .map(|arg| arg.to_token_stream().to_string())
        .collect();
    sig.push_str(&params.join(", "));
    sig.push(')');

    if let syn::ReturnType::Type(_, ref ty) = item.sig.output {
        sig.push_str(" -> ");
        sig.push_str(&ty.to_token_stream().to_string());
    }

    sig
}

/// Reconstruct a human-readable signature for a struct item.
fn struct_signature(item: &syn::ItemStruct) -> String {
    let mut sig = String::new();
    sig.push_str("struct ");
    sig.push_str(&item.ident.to_string());

    if !item.generics.params.is_empty() {
        sig.push_str(&item.generics.to_token_stream().to_string());
    }

    sig
}

/// Reconstruct a human-readable signature for an enum item.
fn enum_signature(item: &syn::ItemEnum) -> String {
    let mut sig = String::new();
    sig.push_str("enum ");
    sig.push_str(&item.ident.to_string());

    if !item.generics.params.is_empty() {
        sig.push_str(&item.generics.to_token_stream().to_string());
    }

    sig
}

/// Reconstruct a human-readable signature for a trait item.
fn trait_signature(item: &syn::ItemTrait) -> String {
    let mut sig = String::new();
    if item.unsafety.is_some() {
        sig.push_str("unsafe ");
    }
    sig.push_str("trait ");
    sig.push_str(&item.ident.to_string());

    if !item.generics.params.is_empty() {
        sig.push_str(&item.generics.to_token_stream().to_string());
    }

    sig
}

/// Reconstruct a human-readable signature for an impl block.
fn impl_signature(item: &syn::ItemImpl) -> String {
    let mut sig = String::new();
    if item.unsafety.is_some() {
        sig.push_str("unsafe ");
    }
    sig.push_str("impl");

    if !item.generics.params.is_empty() {
        sig.push_str(&item.generics.to_token_stream().to_string());
    }

    sig.push(' ');

    if let Some((_, ref trait_path, _)) = item.trait_ {
        sig.push_str(&trait_path.to_token_stream().to_string());
        sig.push_str(" for ");
    }

    sig.push_str(&item.self_ty.to_token_stream().to_string());

    sig
}

/// Reconstruct a human-readable signature for a const item.
fn const_signature(item: &syn::ItemConst) -> String {
    let mut sig = String::new();
    sig.push_str("const ");
    sig.push_str(&item.ident.to_string());
    sig.push_str(": ");
    sig.push_str(&item.ty.to_token_stream().to_string());
    sig
}

/// Reconstruct a human-readable signature for a static item.
fn static_signature(item: &syn::ItemStatic) -> String {
    let mut sig = String::new();
    sig.push_str("static ");
    if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
        sig.push_str("mut ");
    }
    sig.push_str(&item.ident.to_string());
    sig.push_str(": ");
    sig.push_str(&item.ty.to_token_stream().to_string());
    sig
}

/// Reconstruct a human-readable signature for a type alias.
fn type_alias_signature(item: &syn::ItemType) -> String {
    let mut sig = String::new();
    sig.push_str("type ");
    sig.push_str(&item.ident.to_string());

    if !item.generics.params.is_empty() {
        sig.push_str(&item.generics.to_token_stream().to_string());
    }

    sig.push_str(" = ");
    sig.push_str(&item.ty.to_token_stream().to_string());
    sig
}

/// Derive the name for an impl block from its self type.
fn impl_block_name(item: &syn::ItemImpl) -> String {
    if let Some((_, ref trait_path, _)) = item.trait_ {
        let trait_name = trait_path.to_token_stream().to_string();
        let self_name = item.self_ty.to_token_stream().to_string();
        format!("impl {} for {}", trait_name, self_name)
    } else {
        let self_name = item.self_ty.to_token_stream().to_string();
        format!("impl {}", self_name)
    }
}

/// Collect all imported names and their full paths from a `use` tree.
///
/// Given `use std::fs::{read, write};`, this produces:
/// - `("read", "std::fs::read")`
/// - `("write", "std::fs::write")`
///
/// Given `use std::fs::read;`, this produces:
/// - `("read", "std::fs::read")`
///
/// Given `use std::fs::read as file_read;`, this produces:
/// - `("file_read", "std::fs::read")`
fn collect_use_paths(tree: &syn::UseTree, prefix: &str) -> Vec<(String, String)> {
    match tree {
        syn::UseTree::Path(use_path) => {
            let new_prefix = if prefix.is_empty() {
                use_path.ident.to_string()
            } else {
                format!("{}::{}", prefix, use_path.ident)
            };
            collect_use_paths(&use_path.tree, &new_prefix)
        }
        syn::UseTree::Name(use_name) => {
            let name = use_name.ident.to_string();
            let full_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}::{}", prefix, name)
            };
            vec![(name, full_path)]
        }
        syn::UseTree::Rename(use_rename) => {
            let alias = use_rename.rename.to_string();
            let original = use_rename.ident.to_string();
            let full_path = if prefix.is_empty() {
                original
            } else {
                format!("{}::{}", prefix, use_rename.ident)
            };
            vec![(alias, full_path)]
        }
        syn::UseTree::Glob(_) => {
            // `use foo::*` -- record the glob import with the prefix path.
            let full_path = if prefix.is_empty() {
                "*".to_string()
            } else {
                format!("{}::*", prefix)
            };
            vec![("*".to_string(), full_path)]
        }
        syn::UseTree::Group(use_group) => {
            let mut results = Vec::new();
            for item in &use_group.items {
                results.extend(collect_use_paths(item, prefix));
            }
            results
        }
    }
}

/// Extract the last segment name from a syn `Path`.
///
/// For `std::fmt::Display`, returns `"Display"`.
/// For a simple `foo`, returns `"foo"`.
fn path_last_segment(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|seg| seg.ident.to_string())
        .unwrap_or_default()
}

/// Convert a syn `Path` to a full string representation.
///
/// For `std::fmt::Display`, returns `"std::fmt::Display"`.
fn path_to_string(path: &syn::Path) -> String {
    path.segments
        .iter()
        .map(|seg| seg.ident.to_string())
        .collect::<Vec<_>>()
        .join("::")
}

// ---------------------------------------------------------------------------
// Visit implementation
// ---------------------------------------------------------------------------

impl<'ast> Visit<'ast> for SymbolVisitor {
    fn visit_item_fn(&mut self, node: &'ast syn::ItemFn) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.is_async = node.sig.asyncness.is_some();
        attrs.is_unsafe = node.sig.unsafety.is_some();
        attrs.is_extern = node.sig.abi.is_some();
        attrs.is_test = has_test_attribute(&node.attrs);
        attrs.has_unsafe_block = block_contains_unsafe(&node.block);
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        let fn_name = node.sig.ident.to_string();

        self.record(ExtractedSymbol {
            name: fn_name.clone(),
            kind: SymbolKind::Function,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(fn_signature(node)),
            attributes: attrs,
        });

        // Set the current function context for relationship extraction.
        let prev = self.current_function.take();
        self.current_function = Some(fn_name);
        // Continue walking to find nested items and expressions.
        syn::visit::visit_item_fn(self, node);
        self.current_function = prev;
    }

    fn visit_item_struct(&mut self, node: &'ast syn::ItemStruct) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        self.record(ExtractedSymbol {
            name: node.ident.to_string(),
            kind: SymbolKind::Struct,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(struct_signature(node)),
            attributes: attrs,
        });

        syn::visit::visit_item_struct(self, node);
    }

    fn visit_item_enum(&mut self, node: &'ast syn::ItemEnum) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        self.record(ExtractedSymbol {
            name: node.ident.to_string(),
            kind: SymbolKind::Enum,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(enum_signature(node)),
            attributes: attrs,
        });

        syn::visit::visit_item_enum(self, node);
    }

    fn visit_item_trait(&mut self, node: &'ast syn::ItemTrait) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.is_unsafe = node.unsafety.is_some();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        self.record(ExtractedSymbol {
            name: node.ident.to_string(),
            kind: SymbolKind::Trait,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(trait_signature(node)),
            attributes: attrs,
        });

        syn::visit::visit_item_trait(self, node);
    }

    fn visit_item_impl(&mut self, node: &'ast syn::ItemImpl) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.is_unsafe = node.unsafety.is_some();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        let name = impl_block_name(node);
        let self_type_name = node.self_ty.to_token_stream().to_string();

        self.record(ExtractedSymbol {
            name: name.clone(),
            kind: SymbolKind::ImplBlock,
            visibility: Visibility::Private, // impl blocks have no explicit visibility
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(impl_signature(node)),
            attributes: attrs,
        });

        // If this is a trait impl, record an Implements relationship.
        if let Some((_, ref trait_path, _)) = node.trait_ {
            let trait_name = path_last_segment(trait_path);
            let trait_full_path = path_to_string(trait_path);
            self.record_relationship(ExtractedRelationship {
                source_name: self_type_name.clone(),
                target_name: trait_name,
                target_path: Some(trait_full_path),
                kind: RelationshipKind::Implements,
                span_start_line: sl,
                span_start_col: sc,
                span_end_line: el,
                span_end_col: ec,
            });
        }

        // Track the current impl type so methods can reference it.
        let prev = self.current_impl_type.take();
        self.current_impl_type = Some(self_type_name);
        syn::visit::visit_item_impl(self, node);
        self.current_impl_type = prev;
    }

    fn visit_impl_item_fn(&mut self, node: &'ast syn::ImplItemFn) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.is_async = node.sig.asyncness.is_some();
        attrs.is_unsafe = node.sig.unsafety.is_some();
        attrs.is_extern = node.sig.abi.is_some();
        attrs.is_test = has_test_attribute(&node.attrs);
        attrs.has_unsafe_block = block_contains_unsafe(&node.block);
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        let method_name = node.sig.ident.to_string();

        // Build a qualified name for the method context (e.g. "Foo::bar").
        let context_name = match &self.current_impl_type {
            Some(impl_type) => format!("{}::{}", impl_type, method_name),
            None => method_name.clone(),
        };

        self.record(ExtractedSymbol {
            name: method_name,
            kind: SymbolKind::Method,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(method_signature(node)),
            attributes: attrs,
        });

        // Set the current function context for relationship extraction.
        let prev = self.current_function.take();
        self.current_function = Some(context_name);
        syn::visit::visit_impl_item_fn(self, node);
        self.current_function = prev;
    }

    fn visit_item_const(&mut self, node: &'ast syn::ItemConst) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        self.record(ExtractedSymbol {
            name: node.ident.to_string(),
            kind: SymbolKind::Constant,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(const_signature(node)),
            attributes: attrs,
        });

        syn::visit::visit_item_const(self, node);
    }

    fn visit_item_static(&mut self, node: &'ast syn::ItemStatic) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        self.record(ExtractedSymbol {
            name: node.ident.to_string(),
            kind: SymbolKind::Static,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(static_signature(node)),
            attributes: attrs,
        });

        syn::visit::visit_item_static(self, node);
    }

    fn visit_item_type(&mut self, node: &'ast syn::ItemType) {
        let (sl, sc, el, ec) = extract_span(node);
        let mut attrs = SymbolAttributes::empty();
        attrs.doc_comment = extract_doc_comment(&node.attrs);

        self.record(ExtractedSymbol {
            name: node.ident.to_string(),
            kind: SymbolKind::TypeAlias,
            visibility: convert_visibility(&node.vis),
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
            signature: Some(type_alias_signature(node)),
            attributes: attrs,
        });

        syn::visit::visit_item_type(self, node);
    }

    // -------------------------------------------------------------------
    // Relationship extraction
    // -------------------------------------------------------------------

    fn visit_item_use(&mut self, node: &'ast syn::ItemUse) {
        let (sl, sc, el, ec) = extract_span(node);
        let source = self.current_context_name();

        let imports = collect_use_paths(&node.tree, "");
        for (name, full_path) in imports {
            self.record_relationship(ExtractedRelationship {
                source_name: source.clone(),
                target_name: name,
                target_path: Some(full_path),
                kind: RelationshipKind::Imports,
                span_start_line: sl,
                span_start_col: sc,
                span_end_line: el,
                span_end_col: ec,
            });
        }

        syn::visit::visit_item_use(self, node);
    }

    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        let (sl, sc, el, ec) = extract_span(node);
        let source = self.current_context_name();

        // Extract the function name from the call expression.
        // For path-based calls like `foo()` or `module::bar()`, extract the
        // last segment of the path.
        if let syn::Expr::Path(ref expr_path) = *node.func {
            let target_name = path_last_segment(&expr_path.path);
            let target_full_path = path_to_string(&expr_path.path);
            if !target_name.is_empty() {
                self.record_relationship(ExtractedRelationship {
                    source_name: source,
                    target_name,
                    target_path: Some(target_full_path),
                    kind: RelationshipKind::Calls,
                    span_start_line: sl,
                    span_start_col: sc,
                    span_end_line: el,
                    span_end_col: ec,
                });
            }
        }

        // Continue walking to visit nested expressions.
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        let (sl, sc, el, ec) = extract_span(node);
        let source = self.current_context_name();
        let method_name = node.method.to_string();

        self.record_relationship(ExtractedRelationship {
            source_name: source,
            target_name: method_name,
            target_path: None, // Method calls need type inference to resolve
            kind: RelationshipKind::Calls,
            span_start_line: sl,
            span_start_col: sc,
            span_end_line: el,
            span_end_col: ec,
        });

        // Continue walking to visit nested expressions.
        syn::visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_path(&mut self, node: &'ast syn::ExprPath) {
        // Only record path references when inside a function body context.
        // This avoids recording type paths in signatures, etc.
        if self.current_function.is_some() {
            let path = &node.path;
            // Only record multi-segment paths (e.g. `Type::method`) or
            // single-segment paths that look like they reference symbols.
            // Skip `self` and primitives.
            if path.segments.len() >= 2 {
                let (sl, sc, el, ec) = extract_span(node);
                let source = self.current_context_name();
                let target_name = path_last_segment(path);
                let target_full_path = path_to_string(path);

                // Skip `self::` prefixed paths as they are module-relative.
                let first_segment = path.segments.first()
                    .map(|s| s.ident.to_string())
                    .unwrap_or_default();
                if first_segment != "self" && first_segment != "Self" {
                    self.record_relationship(ExtractedRelationship {
                        source_name: source,
                        target_name,
                        target_path: Some(target_full_path),
                        kind: RelationshipKind::References,
                        span_start_line: sl,
                        span_start_col: sc,
                        span_end_line: el,
                        span_end_col: ec,
                    });
                }
            }
        }

        syn::visit::visit_expr_path(self, node);
    }
}

// ---------------------------------------------------------------------------
// Convenience: parse a source string and extract symbols
// ---------------------------------------------------------------------------

/// Parse a Rust source string and extract all symbols.
///
/// This is a convenience function for testing and single-file use cases.
/// Returns `Err` if the source fails to parse.
pub fn extract_symbols_from_source(
    source: &str,
    file_id: FileId,
    module_id: ModuleId,
) -> Result<Vec<ExtractedSymbol>, syn::Error> {
    let file = syn::parse_file(source)?;
    let mut visitor = SymbolVisitor::new(file_id, module_id);
    visitor.visit_file(&file);
    Ok(visitor.into_symbols())
}

/// Parse a Rust source string and extract all symbols and relationships.
///
/// This is a convenience function for testing and single-file use cases.
/// Returns `Err` if the source fails to parse.
pub fn extract_from_source(
    source: &str,
    file_id: FileId,
    module_id: ModuleId,
) -> Result<(Vec<ExtractedSymbol>, Vec<ExtractedRelationship>), syn::Error> {
    let file = syn::parse_file(source)?;
    let mut visitor = SymbolVisitor::new(file_id, module_id);
    visitor.visit_file(&file);
    Ok(visitor.into_results())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use spectron_core::{FileId, ModuleId, SymbolKind, Visibility};

    fn parse_and_extract(source: &str) -> Vec<ExtractedSymbol> {
        extract_symbols_from_source(source, FileId(0), ModuleId(0))
            .expect("source should parse successfully")
    }

    // -----------------------------------------------------------------------
    // Basic symbol extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_function() {
        let source = r#"
            fn hello(x: i32) -> bool {
                true
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "hello");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Private);
        assert!(symbols[0].signature.as_ref().unwrap().contains("fn hello"));
        assert!(symbols[0].signature.as_ref().unwrap().contains("i32"));
        assert!(symbols[0].signature.as_ref().unwrap().contains("bool"));
    }

    #[test]
    fn extract_pub_function() {
        let source = r#"
            pub fn greet(name: &str) {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "greet");
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].visibility, Visibility::Public);
    }

    #[test]
    fn extract_struct() {
        let source = r#"
            pub struct Point {
                pub x: f64,
                pub y: f64,
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Point");
        assert_eq!(symbols[0].kind, SymbolKind::Struct);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert!(symbols[0].signature.as_ref().unwrap().contains("struct Point"));
    }

    #[test]
    fn extract_enum() {
        let source = r#"
            pub enum Color {
                Red,
                Green,
                Blue,
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Color");
        assert_eq!(symbols[0].kind, SymbolKind::Enum);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert!(symbols[0].signature.as_ref().unwrap().contains("enum Color"));
    }

    #[test]
    fn extract_trait() {
        let source = r#"
            pub trait Drawable {
                fn draw(&self);
            }
        "#;
        let symbols = parse_and_extract(source);
        // Trait + the method declaration (trait method is not an ImplItemFn,
        // so we should only get the trait).
        let traits: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Trait).collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Drawable");
        assert!(traits[0].signature.as_ref().unwrap().contains("trait Drawable"));
    }

    #[test]
    fn extract_impl_block_and_method() {
        let source = r#"
            struct Foo;

            impl Foo {
                pub fn bar(&self) -> i32 {
                    42
                }
            }
        "#;
        let symbols = parse_and_extract(source);

        let structs: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Struct).collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Foo");

        let impls: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::ImplBlock).collect();
        assert_eq!(impls.len(), 1);
        assert!(impls[0].name.contains("Foo"));
        assert!(impls[0].signature.as_ref().unwrap().contains("impl"));

        let methods: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Method).collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "bar");
        assert_eq!(methods[0].visibility, Visibility::Public);
        assert!(methods[0].signature.as_ref().unwrap().contains("fn bar"));
    }

    #[test]
    fn extract_const() {
        let source = r#"
            pub const MAX_SIZE: usize = 1024;
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "MAX_SIZE");
        assert_eq!(symbols[0].kind, SymbolKind::Constant);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert!(symbols[0].signature.as_ref().unwrap().contains("const MAX_SIZE"));
        assert!(symbols[0].signature.as_ref().unwrap().contains("usize"));
    }

    #[test]
    fn extract_static() {
        let source = r#"
            static COUNTER: u32 = 0;
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "COUNTER");
        assert_eq!(symbols[0].kind, SymbolKind::Static);
        assert_eq!(symbols[0].visibility, Visibility::Private);
        assert!(symbols[0].signature.as_ref().unwrap().contains("static COUNTER"));
        assert!(symbols[0].signature.as_ref().unwrap().contains("u32"));
    }

    #[test]
    fn extract_static_mut() {
        let source = r#"
            static mut BUFFER: [u8; 256] = [0; 256];
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "BUFFER");
        assert_eq!(symbols[0].kind, SymbolKind::Static);
        assert!(symbols[0].signature.as_ref().unwrap().contains("mut"));
    }

    #[test]
    fn extract_type_alias() {
        let source = r#"
            pub type Result<T> = std::result::Result<T, MyError>;
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Result");
        assert_eq!(symbols[0].kind, SymbolKind::TypeAlias);
        assert_eq!(symbols[0].visibility, Visibility::Public);
        assert!(symbols[0].signature.as_ref().unwrap().contains("type Result"));
    }

    // -----------------------------------------------------------------------
    // Mixed source file (task requirement: function, struct, enum in one file)
    // -----------------------------------------------------------------------

    #[test]
    fn extract_function_struct_enum() {
        let source = r#"
            pub fn process(data: &[u8]) -> bool {
                true
            }

            pub struct Config {
                pub name: String,
                pub value: i32,
            }

            pub enum Status {
                Active,
                Inactive,
                Pending,
            }
        "#;

        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 3);

        let func = symbols.iter().find(|s| s.kind == SymbolKind::Function).unwrap();
        assert_eq!(func.name, "process");
        assert_eq!(func.visibility, Visibility::Public);

        let strukt = symbols.iter().find(|s| s.kind == SymbolKind::Struct).unwrap();
        assert_eq!(strukt.name, "Config");
        assert_eq!(strukt.visibility, Visibility::Public);

        let enu = symbols.iter().find(|s| s.kind == SymbolKind::Enum).unwrap();
        assert_eq!(enu.name, "Status");
        assert_eq!(enu.visibility, Visibility::Public);
    }

    // -----------------------------------------------------------------------
    // Attribute extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_async_function() {
        let source = r#"
            async fn fetch_data() -> Vec<u8> {
                vec![]
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "fetch_data");
        assert!(symbols[0].attributes.is_async);
        assert!(!symbols[0].attributes.is_unsafe);
        assert!(symbols[0].signature.as_ref().unwrap().contains("async"));
    }

    #[test]
    fn extract_unsafe_function() {
        let source = r#"
            unsafe fn dangerous() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "dangerous");
        assert!(symbols[0].attributes.is_unsafe);
        assert!(!symbols[0].attributes.is_async);
        assert!(symbols[0].signature.as_ref().unwrap().contains("unsafe"));
    }

    #[test]
    fn extract_extern_function() {
        let source = r#"
            extern "C" fn callback(x: i32) -> i32 {
                x
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "callback");
        assert!(symbols[0].attributes.is_extern);
        assert!(symbols[0].signature.as_ref().unwrap().contains("extern"));
    }

    #[test]
    fn extract_unsafe_trait() {
        let source = r#"
            unsafe trait Send {}
        "#;
        let symbols = parse_and_extract(source);
        let traits: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Trait).collect();
        assert_eq!(traits.len(), 1);
        assert!(traits[0].attributes.is_unsafe);
    }

    // -----------------------------------------------------------------------
    // Visibility
    // -----------------------------------------------------------------------

    #[test]
    fn extract_pub_crate_visibility() {
        let source = r#"
            pub(crate) fn internal() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].visibility, Visibility::Crate);
    }

    #[test]
    fn extract_pub_super_visibility() {
        let source = r#"
            pub(super) fn parent_visible() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].visibility, Visibility::Restricted);
    }

    // -----------------------------------------------------------------------
    // Impl with trait
    // -----------------------------------------------------------------------

    #[test]
    fn extract_trait_impl() {
        let source = r#"
            struct MyType;
            trait MyTrait {
                fn do_thing(&self);
            }
            impl MyTrait for MyType {
                fn do_thing(&self) {}
            }
        "#;
        let symbols = parse_and_extract(source);

        let impls: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::ImplBlock).collect();
        assert_eq!(impls.len(), 1);
        assert!(impls[0].name.contains("MyTrait"));
        assert!(impls[0].name.contains("MyType"));
        assert!(impls[0].signature.as_ref().unwrap().contains("impl MyTrait for MyType"));

        let methods: Vec<_> = symbols.iter().filter(|s| s.kind == SymbolKind::Method).collect();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].name, "do_thing");
    }

    // -----------------------------------------------------------------------
    // Generic types
    // -----------------------------------------------------------------------

    #[test]
    fn extract_generic_struct() {
        let source = r#"
            pub struct Container<T> {
                value: T,
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "Container");
        assert!(symbols[0].signature.as_ref().unwrap().contains("<"));
    }

    // -----------------------------------------------------------------------
    // Span extraction
    // -----------------------------------------------------------------------

    #[test]
    fn span_values_are_nonzero_for_nonempty_source() {
        let source = r#"fn foo() {}"#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        // syn reports 1-based lines
        assert!(symbols[0].span_start_line >= 1);
    }

    // -----------------------------------------------------------------------
    // Thread-safe accumulator
    // -----------------------------------------------------------------------

    #[test]
    fn accumulator_collects_and_converts() {
        let acc = SymbolAccumulator::new();
        let file_id = FileId(10);
        let module_id = ModuleId(20);

        acc.push(
            file_id,
            module_id,
            ExtractedSymbol {
                name: "test_fn".to_string(),
                kind: SymbolKind::Function,
                visibility: Visibility::Public,
                span_start_line: 1,
                span_start_col: 0,
                span_end_line: 3,
                span_end_col: 1,
                signature: Some("fn test_fn()".to_string()),
                attributes: SymbolAttributes::empty(),
            },
        );

        let id_gen = IdGenerator::new();
        let symbols = acc.into_symbols(&id_gen);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "test_fn");
        assert_eq!(symbols[0].file_id, file_id);
        assert_eq!(symbols[0].module_id, module_id);
        assert_eq!(symbols[0].kind, SymbolKind::Function);
        assert_eq!(symbols[0].span.start_line, 1);
        assert_eq!(symbols[0].span.end_line, 3);
    }

    #[test]
    fn accumulator_thread_safe() {
        use std::sync::Arc;
        use std::thread;

        let acc = Arc::new(SymbolAccumulator::new());
        let mut handles = vec![];

        for i in 0..4u64 {
            let acc = Arc::clone(&acc);
            handles.push(thread::spawn(move || {
                for j in 0..10u64 {
                    acc.push(
                        FileId(i),
                        ModuleId(0),
                        ExtractedSymbol {
                            name: format!("sym_{}_{}", i, j),
                            kind: SymbolKind::Function,
                            visibility: Visibility::Private,
                            span_start_line: 1,
                            span_start_col: 0,
                            span_end_line: 1,
                            span_end_col: 10,
                            signature: None,
                            attributes: SymbolAttributes::empty(),
                        },
                    );
                }
            }));
        }

        for h in handles {
            h.join().expect("thread panicked");
        }

        // We need to unwrap the Arc to call into_entries
        let acc = Arc::try_unwrap(acc).expect("Arc should have single owner");
        let entries = acc.into_entries();
        assert_eq!(entries.len(), 40);
    }

    // -----------------------------------------------------------------------
    // Complex source with all symbol kinds
    // -----------------------------------------------------------------------

    #[test]
    fn extract_all_symbol_kinds() {
        let source = r#"
            pub fn top_level() {}

            pub struct MyStruct {
                field: i32,
            }

            pub enum MyEnum {
                A,
                B,
            }

            pub trait MyTrait {
                fn required(&self);
            }

            impl MyStruct {
                pub fn method(&self) -> i32 {
                    self.field
                }
            }

            pub const MY_CONST: i32 = 42;

            pub static MY_STATIC: &str = "hello";

            pub type MyAlias = Vec<i32>;
        "#;

        let symbols = parse_and_extract(source);

        let kinds: Vec<SymbolKind> = symbols.iter().map(|s| s.kind.clone()).collect();

        assert!(kinds.contains(&SymbolKind::Function), "missing Function");
        assert!(kinds.contains(&SymbolKind::Struct), "missing Struct");
        assert!(kinds.contains(&SymbolKind::Enum), "missing Enum");
        assert!(kinds.contains(&SymbolKind::Trait), "missing Trait");
        assert!(kinds.contains(&SymbolKind::ImplBlock), "missing ImplBlock");
        assert!(kinds.contains(&SymbolKind::Method), "missing Method");
        assert!(kinds.contains(&SymbolKind::Constant), "missing Constant");
        assert!(kinds.contains(&SymbolKind::Static), "missing Static");
        assert!(kinds.contains(&SymbolKind::TypeAlias), "missing TypeAlias");

        // Verify specific names
        let names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"top_level"));
        assert!(names.contains(&"MyStruct"));
        assert!(names.contains(&"MyEnum"));
        assert!(names.contains(&"MyTrait"));
        assert!(names.contains(&"method"));
        assert!(names.contains(&"MY_CONST"));
        assert!(names.contains(&"MY_STATIC"));
        assert!(names.contains(&"MyAlias"));
    }

    // -----------------------------------------------------------------------
    // Empty and edge-case inputs
    // -----------------------------------------------------------------------

    #[test]
    fn empty_source() {
        let symbols = parse_and_extract("");
        assert!(symbols.is_empty());
    }

    #[test]
    fn source_with_only_comments() {
        let source = r#"
            // This is a comment
            /* This is a block comment */
        "#;
        let symbols = parse_and_extract(source);
        assert!(symbols.is_empty());
    }

    #[test]
    fn parse_error_returns_err() {
        let source = "fn broken( {}";
        let result = extract_symbols_from_source(source, FileId(0), ModuleId(0));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Relationship extraction
    // -----------------------------------------------------------------------

    fn parse_and_extract_rels(source: &str) -> (Vec<ExtractedSymbol>, Vec<ExtractedRelationship>) {
        extract_from_source(source, FileId(0), ModuleId(0))
            .expect("source should parse successfully")
    }

    #[test]
    fn extract_use_import_simple() {
        let source = r#"
            use std::fs::read;
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let imports: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_name, "read");
        assert_eq!(
            imports[0].target_path.as_deref(),
            Some("std::fs::read")
        );
    }

    #[test]
    fn extract_use_import_group() {
        let source = r#"
            use std::collections::{HashMap, HashSet};
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let imports: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert_eq!(imports.len(), 2);

        let names: Vec<&str> = imports.iter().map(|r| r.target_name.as_str()).collect();
        assert!(names.contains(&"HashMap"));
        assert!(names.contains(&"HashSet"));
    }

    #[test]
    fn extract_use_import_rename() {
        let source = r#"
            use std::io::Error as IoError;
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let imports: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_name, "IoError");
        assert_eq!(
            imports[0].target_path.as_deref(),
            Some("std::io::Error")
        );
    }

    #[test]
    fn extract_use_import_glob() {
        let source = r#"
            use std::collections::*;
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let imports: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert_eq!(imports.len(), 1);
        assert_eq!(imports[0].target_name, "*");
        assert_eq!(
            imports[0].target_path.as_deref(),
            Some("std::collections::*")
        );
    }

    #[test]
    fn extract_impl_trait_implements() {
        let source = r#"
            struct Foo;

            trait Display {
                fn fmt(&self);
            }

            impl Display for Foo {
                fn fmt(&self) {}
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let implements: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Implements)
            .collect();
        assert_eq!(implements.len(), 1);
        assert_eq!(implements[0].source_name, "Foo");
        assert_eq!(implements[0].target_name, "Display");
    }

    #[test]
    fn extract_impl_trait_with_path() {
        let source = r#"
            struct Foo;

            impl std::fmt::Display for Foo {
                fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                    Ok(())
                }
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let implements: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Implements)
            .collect();
        assert_eq!(implements.len(), 1);
        assert_eq!(implements[0].source_name, "Foo");
        assert_eq!(implements[0].target_name, "Display");
        assert_eq!(
            implements[0].target_path.as_deref(),
            Some("std::fmt::Display")
        );
    }

    #[test]
    fn extract_function_call() {
        let source = r#"
            fn bar() -> i32 { 42 }

            fn foo() {
                bar();
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();
        assert!(
            !calls.is_empty(),
            "expected at least one Calls relationship"
        );

        let bar_call = calls
            .iter()
            .find(|r| r.target_name == "bar")
            .expect("should find a call to bar()");
        assert_eq!(bar_call.source_name, "foo");
    }

    #[test]
    fn extract_method_call() {
        let source = r#"
            fn example() {
                let v = vec![1, 2, 3];
                v.push(4);
                v.len();
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();

        let push_call = calls
            .iter()
            .find(|r| r.target_name == "push")
            .expect("should find a call to push()");
        assert_eq!(push_call.source_name, "example");

        let len_call = calls
            .iter()
            .find(|r| r.target_name == "len")
            .expect("should find a call to len()");
        assert_eq!(len_call.source_name, "example");
    }

    #[test]
    fn extract_qualified_function_call() {
        let source = r#"
            fn example() {
                std::fs::read("test.txt");
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();

        let read_call = calls
            .iter()
            .find(|r| r.target_name == "read")
            .expect("should find a call to std::fs::read()");
        assert_eq!(read_call.source_name, "example");
        assert_eq!(
            read_call.target_path.as_deref(),
            Some("std::fs::read")
        );
    }

    #[test]
    fn extract_path_reference() {
        let source = r#"
            fn example() {
                let _ = std::env::var("HOME");
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        // std::env::var() should produce a Calls relationship from the ExprCall visitor.
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();
        assert!(
            calls.iter().any(|r| r.target_name == "var"),
            "expected a Calls to var"
        );
    }

    #[test]
    fn call_in_method_has_qualified_source() {
        let source = r#"
            struct Foo;

            impl Foo {
                fn do_work(&self) {
                    helper();
                }
            }

            fn helper() {}
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let calls: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls && r.target_name == "helper")
            .collect();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].source_name, "Foo::do_work");
    }

    #[test]
    fn no_relationships_from_empty_source() {
        let (_symbols, rels) = parse_and_extract_rels("");
        assert!(rels.is_empty());
    }

    #[test]
    fn use_at_module_level_has_module_context() {
        let source = r#"
            use std::fs::read;
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let imports: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .collect();
        assert_eq!(imports.len(), 1);
        // Module-level use statement should have "<module>" as source context.
        assert_eq!(imports[0].source_name, "<module>");
    }

    #[test]
    fn inherent_impl_no_implements_relationship() {
        let source = r#"
            struct Foo;

            impl Foo {
                fn bar(&self) {}
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);
        let implements: Vec<_> = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Implements)
            .collect();
        assert!(
            implements.is_empty(),
            "inherent impl should not produce Implements relationship"
        );
    }

    #[test]
    fn relationship_accumulator_basic() {
        let acc = RelationshipAccumulator::new();
        let file_id = FileId(1);
        let module_id = ModuleId(2);

        acc.push(
            file_id,
            module_id,
            ExtractedRelationship {
                source_name: "foo".to_string(),
                target_name: "bar".to_string(),
                target_path: Some("crate::bar".to_string()),
                kind: RelationshipKind::Calls,
                span_start_line: 5,
                span_start_col: 4,
                span_end_line: 5,
                span_end_col: 10,
            },
        );

        let entries = acc.into_entries();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, file_id);
        assert_eq!(entries[0].1, module_id);
        assert_eq!(entries[0].2.source_name, "foo");
        assert_eq!(entries[0].2.target_name, "bar");
        assert_eq!(entries[0].2.kind, RelationshipKind::Calls);
    }

    #[test]
    fn mixed_relationships_from_complex_source() {
        let source = r#"
            use std::fmt::Display;

            struct Foo;

            trait Greet {
                fn greet(&self);
            }

            impl Greet for Foo {
                fn greet(&self) {
                    println!("hello");
                }
            }

            fn helper() -> i32 { 42 }

            fn main() {
                let f = Foo;
                f.greet();
                helper();
            }
        "#;
        let (_symbols, rels) = parse_and_extract_rels(source);

        // Should have Imports, Implements, and Calls relationships.
        let import_count = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Imports)
            .count();
        let implements_count = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Implements)
            .count();
        let calls_count = rels
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .count();

        assert!(import_count >= 1, "expected at least 1 import, got {}", import_count);
        assert!(
            implements_count >= 1,
            "expected at least 1 implements, got {}",
            implements_count
        );
        assert!(calls_count >= 2, "expected at least 2 calls, got {}", calls_count);
    }

    // -----------------------------------------------------------------------
    // Attribute extraction: is_test, has_unsafe_block, doc_comment
    // -----------------------------------------------------------------------

    #[test]
    fn attr_async_fn() {
        let source = r#"
            async fn foo() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "foo");
        assert!(symbols[0].attributes.is_async);
        assert!(!symbols[0].attributes.is_unsafe);
        assert!(!symbols[0].attributes.is_extern);
        assert!(!symbols[0].attributes.is_test);
    }

    #[test]
    fn attr_unsafe_fn() {
        let source = r#"
            unsafe fn bar() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "bar");
        assert!(symbols[0].attributes.is_unsafe);
        assert!(!symbols[0].attributes.is_async);
        assert!(!symbols[0].attributes.is_extern);
        assert!(!symbols[0].attributes.is_test);
    }

    #[test]
    fn attr_test_function() {
        let source = r#"
            #[test]
            fn test_it() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "test_it");
        assert!(symbols[0].attributes.is_test);
        assert!(!symbols[0].attributes.is_async);
        assert!(!symbols[0].attributes.is_unsafe);
    }

    #[test]
    fn attr_tokio_test_function() {
        let source = r#"
            #[tokio::test]
            async fn test_async() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "test_async");
        assert!(symbols[0].attributes.is_test);
        assert!(symbols[0].attributes.is_async);
    }

    #[test]
    fn attr_has_unsafe_block() {
        let source = r#"
            fn safe_wrapper() {
                unsafe {
                    let x = 42;
                }
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "safe_wrapper");
        assert!(symbols[0].attributes.has_unsafe_block);
        // The function itself is not unsafe, only its body contains an unsafe block.
        assert!(!symbols[0].attributes.is_unsafe);
    }

    #[test]
    fn attr_no_unsafe_block() {
        let source = r#"
            fn completely_safe() {
                let x = 42;
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert!(!symbols[0].attributes.has_unsafe_block);
    }

    #[test]
    fn attr_unsafe_fn_with_unsafe_block() {
        let source = r#"
            unsafe fn danger() {
                unsafe {
                    let x = 42;
                }
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].attributes.is_unsafe);
        assert!(symbols[0].attributes.has_unsafe_block);
    }

    #[test]
    fn attr_doc_comment_triple_slash() {
        let source = r#"
            /// This is a doc comment.
            fn documented() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert_eq!(symbols[0].name, "documented");
        let doc = symbols[0].attributes.doc_comment.as_ref()
            .expect("doc_comment should be populated");
        assert!(doc.contains("This is a doc comment"), "got: {}", doc);
    }

    #[test]
    fn attr_doc_comment_multiline() {
        let source = r#"
            /// First line.
            /// Second line.
            fn multi_doc() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        let doc = symbols[0].attributes.doc_comment.as_ref()
            .expect("doc_comment should be populated");
        assert!(doc.contains("First line"), "got: {}", doc);
        assert!(doc.contains("Second line"), "got: {}", doc);
    }

    #[test]
    fn attr_no_doc_comment() {
        let source = r#"
            fn no_docs() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].attributes.doc_comment.is_none());
    }

    #[test]
    fn attr_doc_comment_on_struct() {
        let source = r#"
            /// A point in 2D space.
            pub struct Point {
                pub x: f64,
                pub y: f64,
            }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        let doc = symbols[0].attributes.doc_comment.as_ref()
            .expect("doc_comment should be populated");
        assert!(doc.contains("point in 2D space"), "got: {}", doc);
    }

    #[test]
    fn attr_test_on_method() {
        let source = r#"
            struct Foo;

            impl Foo {
                #[test]
                fn test_method() {}
            }
        "#;
        let symbols = parse_and_extract(source);
        let methods: Vec<_> = symbols.iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert!(methods[0].attributes.is_test);
    }

    #[test]
    fn attr_has_unsafe_block_in_method() {
        let source = r#"
            struct Foo;

            impl Foo {
                fn risky(&self) {
                    unsafe {
                        let _ = 0;
                    }
                }
            }
        "#;
        let symbols = parse_and_extract(source);
        let methods: Vec<_> = symbols.iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert!(methods[0].attributes.has_unsafe_block);
        assert!(!methods[0].attributes.is_unsafe);
    }

    #[test]
    fn attr_async_method() {
        let source = r#"
            struct Foo;

            impl Foo {
                async fn fetch(&self) -> i32 {
                    42
                }
            }
        "#;
        let symbols = parse_and_extract(source);
        let methods: Vec<_> = symbols.iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(methods.len(), 1);
        assert!(methods[0].attributes.is_async);
    }

    #[test]
    fn attr_extern_fn_detected() {
        let source = r#"
            extern "C" fn ffi_func(x: i32) -> i32 { x }
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert!(symbols[0].attributes.is_extern);
    }

    #[test]
    fn attr_non_test_attribute_not_detected_as_test() {
        let source = r#"
            #[inline]
            fn inlined() {}
        "#;
        let symbols = parse_and_extract(source);
        assert_eq!(symbols.len(), 1);
        assert!(!symbols[0].attributes.is_test);
    }
}
