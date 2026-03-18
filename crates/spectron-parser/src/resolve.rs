//! Name resolution and symbol table for Phase 1.
//!
//! This module implements a simplified name resolution strategy:
//!
//! 1. Build a **symbol table**: map from `(ModuleId, name)` to `SymbolId`.
//! 2. For `use` imports, follow the path to find the target symbol.
//! 3. For function calls, look up the function name in:
//!    a. Current module
//!    b. Imported symbols
//!    c. Parent modules (for `pub` symbols)
//! 4. If resolution fails, log a warning and skip the relationship.
//!
//! Full type-aware resolution is a Phase 2 enhancement.

use std::collections::HashMap;

use spectron_core::{
    FileId, ModuleId, Relationship, RelationshipKind, SourceSpan, Symbol, SymbolId, Visibility,
};

use crate::visitor::ExtractedRelationship;

// ---------------------------------------------------------------------------
// Sentinel for unresolved symbols
// ---------------------------------------------------------------------------

/// Sentinel `SymbolId` used for relationships where the target could not
/// be resolved. Consumers should check for this value and treat it as
/// "unresolved / unknown".
pub const UNRESOLVED: SymbolId = SymbolId(u64::MAX);

// ---------------------------------------------------------------------------
// SymbolTable
// ---------------------------------------------------------------------------

/// A lookup table mapping `(ModuleId, name)` to the `SymbolId`(s) defined
/// in that module.
///
/// Multiple symbols can share the same name within a module (e.g. a struct
/// and its impl block), so we store a `Vec<SymbolId>`.
#[derive(Debug)]
pub struct SymbolTable {
    /// Map from (module_id, name) -> list of symbol IDs.
    by_module_name: HashMap<(ModuleId, String), Vec<SymbolId>>,

    /// Global name index: map from name -> list of (SymbolId, ModuleId).
    /// Used as a fallback when module-scoped lookup fails.
    by_name: HashMap<String, Vec<(SymbolId, ModuleId)>>,

    /// Import map: for each module, tracks imported names and their
    /// resolved SymbolIds. Key is (module_id, imported_name).
    imports: HashMap<(ModuleId, String), Vec<SymbolId>>,

    /// Map from module_id -> parent module_id, for walking up the tree.
    module_parents: HashMap<ModuleId, ModuleId>,

    /// Map from symbol_id -> Symbol visibility, for checking pub access
    /// when walking parent modules.
    symbol_visibility: HashMap<SymbolId, Visibility>,
}

impl SymbolTable {
    /// Build a symbol table from a list of symbols and module hierarchy info.
    ///
    /// `module_parents` maps each module to its parent, if any.
    pub fn build(
        symbols: &[Symbol],
        module_parents: &HashMap<ModuleId, ModuleId>,
    ) -> Self {
        let mut by_module_name: HashMap<(ModuleId, String), Vec<SymbolId>> = HashMap::new();
        let mut by_name: HashMap<String, Vec<(SymbolId, ModuleId)>> = HashMap::new();
        let mut symbol_visibility: HashMap<SymbolId, Visibility> = HashMap::new();

        for sym in symbols {
            by_module_name
                .entry((sym.module_id, sym.name.clone()))
                .or_default()
                .push(sym.id);

            by_name
                .entry(sym.name.clone())
                .or_default()
                .push((sym.id, sym.module_id));

            symbol_visibility.insert(sym.id, sym.visibility.clone());
        }

        Self {
            by_module_name,
            by_name,
            imports: HashMap::new(),
            module_parents: module_parents.clone(),
            symbol_visibility,
        }
    }

    /// Look up symbols by name in a specific module.
    pub fn lookup_in_module(&self, module_id: ModuleId, name: &str) -> Option<&[SymbolId]> {
        self.by_module_name
            .get(&(module_id, name.to_string()))
            .map(|v| v.as_slice())
    }

    /// Look up symbols by name globally (across all modules).
    pub fn lookup_global(&self, name: &str) -> Option<&[(SymbolId, ModuleId)]> {
        self.by_name.get(name).map(|v| v.as_slice())
    }

    /// Register an import: in `module_id`, the name `imported_name` resolves
    /// to the given `target_ids`.
    pub fn register_import(
        &mut self,
        module_id: ModuleId,
        imported_name: String,
        target_ids: Vec<SymbolId>,
    ) {
        self.imports
            .entry((module_id, imported_name))
            .or_default()
            .extend(target_ids);
    }

    /// Look up an imported name in a module.
    pub fn lookup_import(&self, module_id: ModuleId, name: &str) -> Option<&[SymbolId]> {
        self.imports
            .get(&(module_id, name.to_string()))
            .map(|v| v.as_slice())
    }

    /// Look up symbols in parent modules, checking for `pub` visibility.
    ///
    /// Walks up the module tree starting from the parent of `module_id`,
    /// looking for symbols named `name` that are visible (Public or Crate).
    pub fn lookup_in_parents(&self, module_id: ModuleId, name: &str) -> Vec<SymbolId> {
        let mut results = Vec::new();
        let mut current = module_id;

        while let Some(&parent) = self.module_parents.get(&current) {
            if let Some(ids) = self.lookup_in_module(parent, name) {
                for &id in ids {
                    // Only include symbols that are visible from child modules.
                    if let Some(vis) = self.symbol_visibility.get(&id) {
                        match vis {
                            Visibility::Public | Visibility::Crate => {
                                results.push(id);
                            }
                            Visibility::Restricted => {
                                // Restricted visibility may or may not include
                                // the child module; be optimistic in Phase 1.
                                results.push(id);
                            }
                            Visibility::Private => {
                                // Private symbols are not visible from children.
                            }
                        }
                    }
                }
            }
            current = parent;
        }

        results
    }

    /// Resolve a name in the context of `module_id`, trying in order:
    /// 1. Current module
    /// 2. Imported symbols
    /// 3. Parent modules (pub symbols)
    /// 4. Global fallback (all modules)
    ///
    /// Returns the resolved SymbolIds, or an empty vec if nothing found.
    pub fn resolve_name(&self, module_id: ModuleId, name: &str) -> Vec<SymbolId> {
        // 1. Current module
        if let Some(ids) = self.lookup_in_module(module_id, name) {
            if !ids.is_empty() {
                return ids.to_vec();
            }
        }

        // 2. Imported symbols
        if let Some(ids) = self.lookup_import(module_id, name) {
            if !ids.is_empty() {
                return ids.to_vec();
            }
        }

        // 3. Parent modules
        let parent_results = self.lookup_in_parents(module_id, name);
        if !parent_results.is_empty() {
            return parent_results;
        }

        // 4. Global fallback -- return all symbols with this name
        if let Some(entries) = self.lookup_global(name) {
            let ids: Vec<SymbolId> = entries.iter().map(|(id, _)| *id).collect();
            if !ids.is_empty() {
                return ids;
            }
        }

        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// resolve_imports
// ---------------------------------------------------------------------------

/// Process import relationships and register them in the symbol table.
///
/// For each `Imports` relationship, attempts to resolve the target path
/// or name to a SymbolId and registers the import mapping.
pub fn resolve_imports(
    table: &mut SymbolTable,
    import_rels: &[(FileId, ModuleId, ExtractedRelationship)],
) {
    for (_file_id, module_id, rel) in import_rels {
        if rel.kind != RelationshipKind::Imports {
            continue;
        }

        // Skip glob imports -- we cannot resolve `*` to specific symbols
        // without full module expansion.
        if rel.target_name == "*" {
            continue;
        }

        // Try to resolve the imported symbol.
        // First, try the full path if available by looking at the last segment.
        let target_name = &rel.target_name;

        // Look up globally by the final name.
        let resolved_ids: Vec<SymbolId> = if let Some(entries) = table.lookup_global(target_name) {
            // If we have a target_path, prefer symbols whose module path
            // matches. Otherwise, take all matches.
            entries.iter().map(|(id, _)| *id).collect()
        } else {
            Vec::new()
        };

        if !resolved_ids.is_empty() {
            table.register_import(*module_id, target_name.clone(), resolved_ids);
        }
        // If we couldn't resolve, that's fine -- it might be an external
        // crate symbol (e.g. `use std::fs::read`). We just don't register
        // it and will log a warning when trying to use it.
    }
}

// ---------------------------------------------------------------------------
// resolve_relationships
// ---------------------------------------------------------------------------

/// Resolve all extracted relationships into fully formed `Relationship` structs
/// with SymbolId-based source and target.
///
/// Takes:
/// - `table`: the symbol table (with imports already registered)
/// - `symbols`: the list of all symbols (to map source names to SymbolIds)
/// - `extracted_rels`: the raw extracted relationships from the visitor
///
/// Returns resolved `Relationship` structs. Relationships that cannot be
/// resolved are either skipped or recorded with the `UNRESOLVED` sentinel.
pub fn resolve_relationships(
    table: &SymbolTable,
    symbols: &[Symbol],
    extracted_rels: &[(FileId, ModuleId, ExtractedRelationship)],
) -> Vec<Relationship> {
    // Build a fast lookup from (module_id, name) -> SymbolId for source
    // resolution. For the source, we also need to handle qualified names
    // like "Foo::bar" for methods.
    let mut source_lookup: HashMap<(ModuleId, String), SymbolId> = HashMap::new();
    for sym in symbols {
        // Register by simple name
        source_lookup
            .entry((sym.module_id, sym.name.clone()))
            .or_insert(sym.id);
    }

    let mut resolved = Vec::new();

    for (file_id, module_id, rel) in extracted_rels {
        // Skip imports -- they are handled separately via resolve_imports.
        // We don't produce Relationship structs for imports in this phase,
        // but we could if needed.
        let span = SourceSpan::new(
            *file_id,
            rel.span_start_line,
            rel.span_start_col,
            rel.span_end_line,
            rel.span_end_col,
        );

        // Resolve the source symbol.
        let source_id = resolve_source(
            &source_lookup,
            symbols,
            *module_id,
            &rel.source_name,
        );

        // Resolve the target symbol.
        let target_ids = match rel.kind {
            RelationshipKind::Imports => {
                // For imports, try to resolve through the symbol table.
                let ids = table.resolve_name(*module_id, &rel.target_name);
                if ids.is_empty() {
                    tracing::debug!(
                        target_name = %rel.target_name,
                        module_id = %module_id,
                        "could not resolve import target (likely external crate)"
                    );
                }
                ids
            }
            RelationshipKind::Calls => {
                let ids = table.resolve_name(*module_id, &rel.target_name);
                if ids.is_empty() {
                    tracing::debug!(
                        source = %rel.source_name,
                        target = %rel.target_name,
                        module_id = %module_id,
                        "could not resolve call target"
                    );
                }
                ids
            }
            RelationshipKind::Implements => {
                let ids = table.resolve_name(*module_id, &rel.target_name);
                if ids.is_empty() {
                    tracing::debug!(
                        source = %rel.source_name,
                        target = %rel.target_name,
                        module_id = %module_id,
                        "could not resolve implements target"
                    );
                }
                ids
            }
            RelationshipKind::References => {
                let ids = table.resolve_name(*module_id, &rel.target_name);
                if ids.is_empty() {
                    tracing::debug!(
                        source = %rel.source_name,
                        target = %rel.target_name,
                        module_id = %module_id,
                        "could not resolve reference target"
                    );
                }
                ids
            }
            RelationshipKind::Contains | RelationshipKind::DependsOn => {
                // These relationship kinds are handled at a higher level.
                continue;
            }
        };

        if let Some(src_id) = source_id {
            if target_ids.is_empty() {
                // Record with UNRESOLVED sentinel.
                resolved.push(Relationship::with_span(
                    src_id,
                    UNRESOLVED,
                    rel.kind.clone(),
                    span,
                ));
            } else {
                // Create a relationship for each resolved target.
                for &target_id in &target_ids {
                    resolved.push(Relationship::with_span(
                        src_id,
                        target_id,
                        rel.kind.clone(),
                        span.clone(),
                    ));
                }
            }
        } else {
            // Source could not be resolved -- this typically means
            // the source is "<module>" (module-level context) or a
            // qualified name we couldn't map. Log and skip.
            if rel.source_name != "<module>" {
                tracing::debug!(
                    source = %rel.source_name,
                    target = %rel.target_name,
                    "could not resolve source symbol"
                );
            }

            // For module-level imports, we can still record with UNRESOLVED source
            // if there's a resolved target.
            // But generally, skip unresolvable source relationships.
        }
    }

    resolved
}

/// Resolve a source name to a SymbolId.
///
/// Handles:
/// - Simple names: "foo" -> look up (module_id, "foo")
/// - Qualified names: "Foo::bar" -> look up (module_id, "bar") for methods
/// - Module context: "<module>" -> None (module-level, no specific symbol)
fn resolve_source(
    source_lookup: &HashMap<(ModuleId, String), SymbolId>,
    symbols: &[Symbol],
    module_id: ModuleId,
    source_name: &str,
) -> Option<SymbolId> {
    if source_name == "<module>" {
        return None;
    }

    // Try direct lookup by name in the module.
    if let Some(&id) = source_lookup.get(&(module_id, source_name.to_string())) {
        return Some(id);
    }

    // If the name contains "::", it's a qualified name (e.g. "Foo::bar").
    // Try looking up the last segment as a method name.
    if let Some(last_segment) = source_name.rsplit("::").next() {
        if last_segment != source_name {
            if let Some(&id) = source_lookup.get(&(module_id, last_segment.to_string())) {
                return Some(id);
            }
        }
    }

    // Fallback: search all symbols in this module for a matching name.
    for sym in symbols {
        if sym.module_id == module_id && sym.name == source_name {
            return Some(sym.id);
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use spectron_core::{
        FileId, IdGenerator, ModuleId, SourceSpan, Symbol, SymbolAttributes, SymbolKind,
        Visibility,
    };
    use crate::visitor::extract_from_source;

    /// Helper: create a minimal symbol for testing.
    fn make_symbol(
        id_gen: &IdGenerator,
        name: &str,
        kind: SymbolKind,
        module_id: ModuleId,
        file_id: FileId,
        visibility: Visibility,
    ) -> Symbol {
        Symbol {
            id: id_gen.next_symbol(),
            name: name.to_string(),
            kind,
            module_id,
            file_id,
            span: SourceSpan::new(file_id, 1, 0, 1, 10),
            visibility,
            signature: None,
            attributes: SymbolAttributes::empty(),
        }
    }

    // -------------------------------------------------------------------
    // Symbol table basics
    // -------------------------------------------------------------------

    #[test]
    fn symbol_table_lookup_in_module() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let sym = make_symbol(&gen, "foo", SymbolKind::Function, mod_id, file_id, Visibility::Public);
        let sym_id = sym.id;

        let parents = HashMap::new();
        let table = SymbolTable::build(&[sym], &parents);

        let result = table.lookup_in_module(mod_id, "foo");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &[sym_id]);
    }

    #[test]
    fn symbol_table_lookup_missing() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let sym = make_symbol(&gen, "foo", SymbolKind::Function, mod_id, file_id, Visibility::Public);

        let parents = HashMap::new();
        let table = SymbolTable::build(&[sym], &parents);

        let result = table.lookup_in_module(mod_id, "bar");
        assert!(result.is_none());
    }

    #[test]
    fn symbol_table_lookup_global() {
        let gen = IdGenerator::new();
        let mod1 = gen.next_module();
        let mod2 = gen.next_module();
        let file_id = gen.next_file();

        let sym1 = make_symbol(&gen, "foo", SymbolKind::Function, mod1, file_id, Visibility::Public);
        let sym2 = make_symbol(&gen, "foo", SymbolKind::Function, mod2, file_id, Visibility::Public);

        let parents = HashMap::new();
        let table = SymbolTable::build(&[sym1.clone(), sym2.clone()], &parents);

        let result = table.lookup_global("foo");
        assert!(result.is_some());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 2);
        let ids: Vec<SymbolId> = entries.iter().map(|(id, _)| *id).collect();
        assert!(ids.contains(&sym1.id));
        assert!(ids.contains(&sym2.id));
    }

    #[test]
    fn symbol_table_imports() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let sym = make_symbol(&gen, "bar", SymbolKind::Function, ModuleId(999), file_id, Visibility::Public);
        let sym_id = sym.id;

        let parents = HashMap::new();
        let mut table = SymbolTable::build(&[sym], &parents);

        // Register import: in mod_id, "bar" resolves to sym_id.
        table.register_import(mod_id, "bar".to_string(), vec![sym_id]);

        let result = table.lookup_import(mod_id, "bar");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), &[sym_id]);
    }

    #[test]
    fn symbol_table_parent_lookup() {
        let gen = IdGenerator::new();
        let parent_mod = gen.next_module();
        let child_mod = gen.next_module();
        let file_id = gen.next_file();

        let pub_sym = make_symbol(
            &gen, "helper", SymbolKind::Function, parent_mod, file_id, Visibility::Public,
        );
        let priv_sym = make_symbol(
            &gen, "secret", SymbolKind::Function, parent_mod, file_id, Visibility::Private,
        );

        let mut parents = HashMap::new();
        parents.insert(child_mod, parent_mod);

        let table = SymbolTable::build(&[pub_sym.clone(), priv_sym.clone()], &parents);

        // Public symbol should be found via parent lookup.
        let results = table.lookup_in_parents(child_mod, "helper");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0], pub_sym.id);

        // Private symbol should NOT be found via parent lookup.
        let results = table.lookup_in_parents(child_mod, "secret");
        assert!(results.is_empty());
    }

    // -------------------------------------------------------------------
    // resolve_name integration
    // -------------------------------------------------------------------

    #[test]
    fn resolve_name_current_module_first() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let sym = make_symbol(&gen, "foo", SymbolKind::Function, mod_id, file_id, Visibility::Private);
        let sym_id = sym.id;

        let parents = HashMap::new();
        let table = SymbolTable::build(&[sym], &parents);

        let result = table.resolve_name(mod_id, "foo");
        assert_eq!(result, vec![sym_id]);
    }

    #[test]
    fn resolve_name_via_import() {
        let gen = IdGenerator::new();
        let mod_a = gen.next_module();
        let mod_b = gen.next_module();
        let file_id = gen.next_file();

        // Symbol "bar" is in module B.
        let sym = make_symbol(&gen, "bar", SymbolKind::Function, mod_b, file_id, Visibility::Public);
        let sym_id = sym.id;

        let parents = HashMap::new();
        let mut table = SymbolTable::build(&[sym], &parents);

        // Module A imports "bar" from module B.
        table.register_import(mod_a, "bar".to_string(), vec![sym_id]);

        // Resolving "bar" in module A should find it via import.
        let result = table.resolve_name(mod_a, "bar");
        assert_eq!(result, vec![sym_id]);
    }

    #[test]
    fn resolve_name_unresolved() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let sym = make_symbol(&gen, "foo", SymbolKind::Function, mod_id, file_id, Visibility::Private);

        let parents = HashMap::new();
        let table = SymbolTable::build(&[sym], &parents);

        let result = table.resolve_name(mod_id, "nonexistent");
        assert!(result.is_empty());
    }

    // -------------------------------------------------------------------
    // End-to-end: two functions in same module, one calling the other
    // -------------------------------------------------------------------

    #[test]
    fn resolve_calls_same_module() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let source = r#"
            fn bar() -> i32 { 42 }

            fn foo() {
                bar();
            }
        "#;

        let (extracted_syms, extracted_rels) =
            extract_from_source(source, file_id, mod_id)
                .expect("parse should succeed");

        // Convert extracted symbols to full Symbol structs.
        let symbols: Vec<Symbol> = extracted_syms
            .into_iter()
            .map(|ext| Symbol {
                id: gen.next_symbol(),
                name: ext.name,
                kind: ext.kind,
                module_id: mod_id,
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

        let parents = HashMap::new();
        let table = SymbolTable::build(&symbols, &parents);

        let rels_with_context: Vec<(FileId, ModuleId, ExtractedRelationship)> = extracted_rels
            .into_iter()
            .map(|rel| (file_id, mod_id, rel))
            .collect();

        let resolved = resolve_relationships(&table, &symbols, &rels_with_context);

        // Find the "Calls" relationship from foo -> bar.
        let foo_sym = symbols.iter().find(|s| s.name == "foo").expect("should have foo");
        let bar_sym = symbols.iter().find(|s| s.name == "bar").expect("should have bar");

        let calls: Vec<&Relationship> = resolved
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();

        assert!(
            !calls.is_empty(),
            "expected at least one Calls relationship, got none"
        );

        let foo_calls_bar = calls.iter().find(|r| {
            r.source == foo_sym.id && r.target == bar_sym.id
        });

        assert!(
            foo_calls_bar.is_some(),
            "expected foo -> bar Calls relationship; calls found: {:?}",
            calls.iter().map(|r| (r.source, r.target)).collect::<Vec<_>>()
        );
    }

    // -------------------------------------------------------------------
    // Function calling imported symbol
    // -------------------------------------------------------------------

    #[test]
    fn resolve_calls_imported_symbol() {
        let gen = IdGenerator::new();
        let mod_a = gen.next_module();
        let mod_b = gen.next_module();
        let file_a = gen.next_file();
        let file_b = gen.next_file();

        // Module B has function "helper".
        let helper_sym = make_symbol(
            &gen, "helper", SymbolKind::Function, mod_b, file_b, Visibility::Public,
        );
        let helper_id = helper_sym.id;

        // Module A has function "caller" that calls "helper".
        let source_a = r#"
            use other::helper;

            fn caller() {
                helper();
            }
        "#;

        let (extracted_syms_a, extracted_rels_a) =
            extract_from_source(source_a, file_a, mod_a)
                .expect("parse should succeed");

        // Build symbols for module A.
        let mut all_symbols = vec![helper_sym];
        for ext in extracted_syms_a {
            all_symbols.push(Symbol {
                id: gen.next_symbol(),
                name: ext.name,
                kind: ext.kind,
                module_id: mod_a,
                file_id: file_a,
                span: SourceSpan::new(
                    file_a,
                    ext.span_start_line,
                    ext.span_start_col,
                    ext.span_end_line,
                    ext.span_end_col,
                ),
                visibility: ext.visibility,
                signature: ext.signature,
                attributes: ext.attributes,
            });
        }

        let parents = HashMap::new();
        let mut table = SymbolTable::build(&all_symbols, &parents);

        // Process imports to register "helper" in module A's import map.
        let rels_with_context: Vec<(FileId, ModuleId, ExtractedRelationship)> = extracted_rels_a
            .iter()
            .cloned()
            .map(|rel| (file_a, mod_a, rel))
            .collect();

        let import_rels: Vec<_> = rels_with_context
            .iter()
            .filter(|(_, _, rel)| rel.kind == RelationshipKind::Imports)
            .cloned()
            .collect();

        resolve_imports(&mut table, &import_rels);

        // Now resolve all relationships.
        let resolved = resolve_relationships(&table, &all_symbols, &rels_with_context);

        // Find the caller symbol.
        let caller_sym = all_symbols
            .iter()
            .find(|s| s.name == "caller")
            .expect("should have caller");

        // Find Calls relationships.
        let calls: Vec<&Relationship> = resolved
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();

        assert!(
            !calls.is_empty(),
            "expected at least one Calls relationship"
        );

        let caller_calls_helper = calls.iter().find(|r| {
            r.source == caller_sym.id && r.target == helper_id
        });

        assert!(
            caller_calls_helper.is_some(),
            "expected caller -> helper Calls relationship; calls: {:?}",
            calls.iter().map(|r| (r.source, r.target)).collect::<Vec<_>>()
        );
    }

    // -------------------------------------------------------------------
    // Function calling unknown name -> graceful handling
    // -------------------------------------------------------------------

    #[test]
    fn resolve_calls_unknown_name() {
        let gen = IdGenerator::new();
        let mod_id = gen.next_module();
        let file_id = gen.next_file();

        let source = r#"
            fn foo() {
                unknown_function();
            }
        "#;

        let (extracted_syms, extracted_rels) =
            extract_from_source(source, file_id, mod_id)
                .expect("parse should succeed");

        let symbols: Vec<Symbol> = extracted_syms
            .into_iter()
            .map(|ext| Symbol {
                id: gen.next_symbol(),
                name: ext.name,
                kind: ext.kind,
                module_id: mod_id,
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

        let parents = HashMap::new();
        let table = SymbolTable::build(&symbols, &parents);

        let rels_with_context: Vec<(FileId, ModuleId, ExtractedRelationship)> = extracted_rels
            .into_iter()
            .map(|rel| (file_id, mod_id, rel))
            .collect();

        let resolved = resolve_relationships(&table, &symbols, &rels_with_context);

        // The call to unknown_function should produce a relationship with
        // the UNRESOLVED sentinel target.
        let calls: Vec<&Relationship> = resolved
            .iter()
            .filter(|r| r.kind == RelationshipKind::Calls)
            .collect();

        assert!(
            !calls.is_empty(),
            "expected at least one Calls relationship (even if unresolved)"
        );

        let foo_sym = symbols.iter().find(|s| s.name == "foo").expect("should have foo");

        let unresolved_call = calls.iter().find(|r| {
            r.source == foo_sym.id && r.target == UNRESOLVED
        });

        assert!(
            unresolved_call.is_some(),
            "expected unresolved call from foo -> UNRESOLVED; calls: {:?}",
            calls.iter().map(|r| (r.source, r.target)).collect::<Vec<_>>()
        );
    }

    // -------------------------------------------------------------------
    // UNRESOLVED sentinel value
    // -------------------------------------------------------------------

    #[test]
    fn unresolved_sentinel_is_max_u64() {
        assert_eq!(UNRESOLVED, SymbolId(u64::MAX));
    }

    // -------------------------------------------------------------------
    // Multiple symbols with same name across modules
    // -------------------------------------------------------------------

    #[test]
    fn resolve_name_prefers_current_module() {
        let gen = IdGenerator::new();
        let mod_a = gen.next_module();
        let mod_b = gen.next_module();
        let file_id = gen.next_file();

        let sym_a = make_symbol(&gen, "foo", SymbolKind::Function, mod_a, file_id, Visibility::Private);
        let sym_b = make_symbol(&gen, "foo", SymbolKind::Function, mod_b, file_id, Visibility::Public);

        let parents = HashMap::new();
        let table = SymbolTable::build(&[sym_a.clone(), sym_b.clone()], &parents);

        // Resolving "foo" in mod_a should find sym_a (current module wins).
        let result = table.resolve_name(mod_a, "foo");
        assert_eq!(result, vec![sym_a.id]);

        // Resolving "foo" in mod_b should find sym_b.
        let result = table.resolve_name(mod_b, "foo");
        assert_eq!(result, vec![sym_b.id]);
    }

    // -------------------------------------------------------------------
    // Parent module resolution
    // -------------------------------------------------------------------

    #[test]
    fn resolve_name_through_parent() {
        let gen = IdGenerator::new();
        let parent_mod = gen.next_module();
        let child_mod = gen.next_module();
        let file_id = gen.next_file();

        let pub_fn = make_symbol(
            &gen, "parent_helper", SymbolKind::Function, parent_mod, file_id, Visibility::Public,
        );

        let mut parents = HashMap::new();
        parents.insert(child_mod, parent_mod);

        let table = SymbolTable::build(&[pub_fn.clone()], &parents);

        // Resolving in child_mod should find the parent's public symbol.
        let result = table.resolve_name(child_mod, "parent_helper");
        assert_eq!(result, vec![pub_fn.id]);
    }
}
