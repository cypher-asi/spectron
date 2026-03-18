//! Entrypoint detection for the analysis engine.
//!
//! An entrypoint is a function that serves as a program entry or request
//! handler. This module walks all symbols and identifies entrypoints using
//! multiple heuristics:
//!
//! 1. **main function**: `SymbolKind::Function` named `"main"` in a crate root module.
//! 2. **Attribute-based**: `#[tokio::main]`, `#[async_std::main]`, `#[actix_web::main]`.
//! 3. **Handler patterns**: `#[get(...)]`, `#[post(...)]`, `#[put(...)]`,
//!    `#[delete(...)]`, `#[handler]`.
//! 4. **Test functions**: Symbols with `is_test = true`.
//! 5. **CLI commands**: `#[command]` attribute.
//! 6. **Zero-caller functions**: `SymbolKind::Function` with no callers in the
//!    call graph (potential top-level entries).

use std::collections::{HashMap, HashSet};

use spectron_core::{ModuleId, ModuleInfo, Symbol, SymbolId, SymbolKind};
use spectron_graph::CallGraphData;

// ---------------------------------------------------------------------------
// Attribute patterns for entrypoint detection
// ---------------------------------------------------------------------------

/// Attribute paths that indicate an async runtime entrypoint.
const ASYNC_MAIN_ATTRS: &[&str] = &[
    "tokio::main",
    "async_std::main",
    "actix_web::main",
];

/// Attribute path prefixes that indicate HTTP handler patterns.
/// We match these as prefixes so that `get`, `get(...)` etc. all match.
const HANDLER_ATTR_PREFIXES: &[&str] = &[
    "get",
    "post",
    "put",
    "delete",
    "handler",
];

/// Attribute paths that indicate a CLI command entrypoint.
const CLI_COMMAND_ATTRS: &[&str] = &[
    "command",
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect all entrypoint symbols.
///
/// Returns a deduplicated list of `SymbolId`s identified as entrypoints.
/// The detection rules are applied in order:
///
/// 1. `main` function in a crate root module
/// 2. Async main attributes (`#[tokio::main]`, etc.)
/// 3. HTTP handler attributes (`#[get(...)]`, `#[post(...)]`, etc.)
/// 4. Test functions (`is_test = true`)
/// 5. CLI command attributes (`#[command]`)
/// 6. Functions with zero callers in the call graph
pub fn detect_entrypoints(
    symbols: &HashMap<SymbolId, Symbol>,
    modules: &HashMap<ModuleId, ModuleInfo>,
    call_graph_data: &CallGraphData,
) -> Vec<SymbolId> {
    let mut entrypoints: HashSet<SymbolId> = HashSet::new();

    // Build a set of root module IDs (modules with no parent).
    let root_module_ids: HashSet<ModuleId> = modules
        .values()
        .filter(|m| m.parent.is_none())
        .map(|m| m.id)
        .collect();

    for (sym_id, symbol) in symbols {
        let is_function = symbol.kind == SymbolKind::Function;
        let is_method = symbol.kind == SymbolKind::Method;
        let is_callable = is_function || is_method;

        // Rule 1: main function in a crate root module.
        if is_function && symbol.name == "main" && root_module_ids.contains(&symbol.module_id) {
            tracing::debug!(
                symbol_id = sym_id.0,
                "detected entrypoint: main function in root module"
            );
            entrypoints.insert(*sym_id);
            continue;
        }

        // Rule 2: Async main attributes.
        if is_callable && has_any_attribute(&symbol.attributes.attribute_paths, ASYNC_MAIN_ATTRS) {
            tracing::debug!(
                symbol_id = sym_id.0,
                name = %symbol.name,
                "detected entrypoint: async main attribute"
            );
            entrypoints.insert(*sym_id);
            continue;
        }

        // Rule 3: Handler pattern attributes.
        if is_callable && has_any_attribute_prefix(&symbol.attributes.attribute_paths, HANDLER_ATTR_PREFIXES) {
            tracing::debug!(
                symbol_id = sym_id.0,
                name = %symbol.name,
                "detected entrypoint: handler attribute"
            );
            entrypoints.insert(*sym_id);
            continue;
        }

        // Rule 4: Test functions.
        if symbol.attributes.is_test {
            tracing::debug!(
                symbol_id = sym_id.0,
                name = %symbol.name,
                "detected entrypoint: test function"
            );
            entrypoints.insert(*sym_id);
            continue;
        }

        // Rule 5: CLI command attributes.
        if is_callable && has_any_attribute(&symbol.attributes.attribute_paths, CLI_COMMAND_ATTRS) {
            tracing::debug!(
                symbol_id = sym_id.0,
                name = %symbol.name,
                "detected entrypoint: CLI command attribute"
            );
            entrypoints.insert(*sym_id);
            continue;
        }

        // Rule 6: Functions with zero callers in the call graph.
        if is_function {
            let has_callers = call_graph_data
                .callers
                .get(sym_id)
                .map(|callers| !callers.is_empty())
                .unwrap_or(false);

            if !has_callers {
                tracing::debug!(
                    symbol_id = sym_id.0,
                    name = %symbol.name,
                    "detected entrypoint: function with zero callers"
                );
                entrypoints.insert(*sym_id);
            }
        }
    }

    let mut result: Vec<SymbolId> = entrypoints.into_iter().collect();
    // Sort for deterministic output.
    result.sort_by_key(|id| id.0);
    result
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Check if any of the symbol's attribute paths exactly match one of the
/// given patterns.
fn has_any_attribute(attribute_paths: &[String], patterns: &[&str]) -> bool {
    attribute_paths
        .iter()
        .any(|attr| patterns.iter().any(|p| attr == p))
}

/// Check if any of the symbol's attribute paths start with one of the given
/// prefixes. This handles patterns like `get(...)` matching the prefix `"get"`.
fn has_any_attribute_prefix(attribute_paths: &[String], prefixes: &[&str]) -> bool {
    attribute_paths.iter().any(|attr| {
        prefixes.iter().any(|prefix| {
            attr == prefix || attr.starts_with(&format!("{}(", prefix))
        })
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::HashMap;

    use spectron_core::{
        FileId, ModuleId, ModuleInfo, ModulePath, SourceSpan, Symbol,
        SymbolAttributes, SymbolId, SymbolKind, Visibility,
    };
    use spectron_graph::CallGraphData;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn make_symbol(
        id: SymbolId,
        name: &str,
        kind: SymbolKind,
        module_id: ModuleId,
        attributes: SymbolAttributes,
    ) -> Symbol {
        let file_id = FileId(0);
        Symbol {
            id,
            name: name.to_owned(),
            kind,
            module_id,
            file_id,
            span: SourceSpan::new(file_id, 1, 0, 10, 0),
            visibility: Visibility::Public,
            signature: Some(format!("fn {}()", name)),
            attributes,
        }
    }

    fn empty_call_graph_data() -> CallGraphData {
        CallGraphData {
            callers: HashMap::new(),
            callees: HashMap::new(),
        }
    }

    fn make_root_module(id: ModuleId) -> ModuleInfo {
        ModuleInfo::new(
            id,
            "my_crate",
            ModulePath::new("my_crate"),
            None,
            None, // parent = None -> root module
        )
    }

    fn make_child_module(id: ModuleId, parent: ModuleId) -> ModuleInfo {
        ModuleInfo::new(
            id,
            "child",
            ModulePath::new("my_crate::child"),
            None,
            Some(parent),
        )
    }

    // -----------------------------------------------------------------------
    // Test: main function in root module is detected as entrypoint
    // -----------------------------------------------------------------------

    #[test]
    fn main_in_root_module_is_entrypoint() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let symbol = make_symbol(
            sym_id,
            "main",
            SymbolKind::Function,
            root_mod,
            SymbolAttributes::empty(),
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "main function in root module should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: main function in child module is NOT detected by rule 1
    //       (but may still be detected by rule 6 if no callers)
    // -----------------------------------------------------------------------

    #[test]
    fn main_in_child_module_not_detected_by_name_rule() {
        let root_mod = ModuleId(0);
        let child_mod = ModuleId(1);
        let sym_main = SymbolId(1);
        let sym_caller = SymbolId(2);

        // main is in a child module
        let main_symbol = make_symbol(
            sym_main,
            "main",
            SymbolKind::Function,
            child_mod,
            SymbolAttributes::empty(),
        );

        // Give it a caller so rule 6 does not fire
        let caller_symbol = make_symbol(
            sym_caller,
            "caller",
            SymbolKind::Function,
            root_mod,
            SymbolAttributes::empty(),
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_main, main_symbol);
        symbols.insert(sym_caller, caller_symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));
        modules.insert(child_mod, make_child_module(child_mod, root_mod));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_main, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_main]);
                m.insert(sym_main, vec![]);
                m
            },
        };

        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        // sym_main should NOT be detected (it is in a child module and has callers)
        assert!(
            !entrypoints.contains(&sym_main),
            "main in child module with callers should not be an entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: tokio::main attribute is detected
    // -----------------------------------------------------------------------

    #[test]
    fn tokio_main_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec!["tokio::main".to_owned()];

        let symbol = make_symbol(
            sym_id,
            "main",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[tokio::main] should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: async_std::main attribute is detected
    // -----------------------------------------------------------------------

    #[test]
    fn async_std_main_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec!["async_std::main".to_owned()];

        let symbol = make_symbol(
            sym_id,
            "run",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[async_std::main] should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: actix_web::main attribute is detected
    // -----------------------------------------------------------------------

    #[test]
    fn actix_web_main_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec!["actix_web::main".to_owned()];

        let symbol = make_symbol(
            sym_id,
            "main",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[actix_web::main] should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: handler attribute patterns are detected
    // -----------------------------------------------------------------------

    #[test]
    fn handler_get_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec![r#"get("/api/users")"#.to_owned()];

        let symbol = make_symbol(
            sym_id,
            "get_users",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[get(...)] should be detected as entrypoint"
        );
    }

    #[test]
    fn handler_post_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec![r#"post("/api/users")"#.to_owned()];

        let symbol = make_symbol(
            sym_id,
            "create_user",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[post(...)] should be detected as entrypoint"
        );
    }

    #[test]
    fn handler_bare_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec!["handler".to_owned()];

        let symbol = make_symbol(
            sym_id,
            "handle_request",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[handler] should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: test functions (is_test = true) are detected
    // -----------------------------------------------------------------------

    #[test]
    fn test_function_detected_as_entrypoint() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.is_test = true;

        let symbol = make_symbol(
            sym_id,
            "test_something",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with is_test=true should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: CLI command attribute is detected
    // -----------------------------------------------------------------------

    #[test]
    fn command_attribute_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec!["command".to_owned()];

        let symbol = make_symbol(
            sym_id,
            "do_action",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with #[command] should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: function with zero callers is detected as entrypoint
    // -----------------------------------------------------------------------

    #[test]
    fn zero_callers_function_detected() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        let symbol = make_symbol(
            sym_id,
            "orphan_function",
            SymbolKind::Function,
            root_mod,
            SymbolAttributes::empty(),
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        // Empty call graph data = no callers for anyone
        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            entrypoints.contains(&sym_id),
            "function with zero callers should be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: function WITH callers is NOT an entrypoint (via rule 6)
    // -----------------------------------------------------------------------

    #[test]
    fn function_with_callers_not_entrypoint() {
        let root_mod = ModuleId(0);
        let sym_callee = SymbolId(1);
        let sym_caller = SymbolId(2);

        // callee is a regular function with no special attributes
        let callee = make_symbol(
            sym_callee,
            "helper",
            SymbolKind::Function,
            root_mod,
            SymbolAttributes::empty(),
        );

        // caller calls callee
        let caller = make_symbol(
            sym_caller,
            "do_work",
            SymbolKind::Function,
            root_mod,
            SymbolAttributes::empty(),
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_callee, callee);
        symbols.insert(sym_caller, caller);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_callee, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_callee]);
                m.insert(sym_callee, vec![]);
                m
            },
        };

        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        // sym_callee has callers, so it should NOT be detected by rule 6.
        // It also has no special name or attributes.
        assert!(
            !entrypoints.contains(&sym_callee),
            "function with callers and no special attributes should not be an entrypoint"
        );

        // sym_caller has zero callers, so it IS detected by rule 6.
        assert!(
            entrypoints.contains(&sym_caller),
            "function with zero callers should be an entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: structs and non-function symbols are not entrypoints
    // -----------------------------------------------------------------------

    #[test]
    fn non_function_symbols_not_entrypoints() {
        let root_mod = ModuleId(0);
        let sym_struct = SymbolId(1);

        let struct_symbol = make_symbol(
            sym_struct,
            "MyStruct",
            SymbolKind::Struct,
            root_mod,
            SymbolAttributes::empty(),
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_struct, struct_symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(
            !entrypoints.contains(&sym_struct),
            "struct should not be detected as entrypoint"
        );
    }

    // -----------------------------------------------------------------------
    // Test: entrypoints are deduplicated
    // -----------------------------------------------------------------------

    #[test]
    fn entrypoints_are_deduplicated() {
        let root_mod = ModuleId(0);
        let sym_id = SymbolId(1);

        // This symbol matches multiple rules: main function in root module,
        // tokio::main attribute, AND zero callers. It should appear only once.
        let mut attrs = SymbolAttributes::empty();
        attrs.attribute_paths = vec!["tokio::main".to_owned()];

        let symbol = make_symbol(
            sym_id,
            "main",
            SymbolKind::Function,
            root_mod,
            attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        let count = entrypoints.iter().filter(|id| **id == sym_id).count();
        assert_eq!(count, 1, "entrypoint should appear exactly once even when matching multiple rules");
    }

    // -----------------------------------------------------------------------
    // Test: multiple entrypoints detected
    // -----------------------------------------------------------------------

    #[test]
    fn multiple_entrypoints_detected() {
        let root_mod = ModuleId(0);
        let sym_main = SymbolId(1);
        let sym_test = SymbolId(2);
        let sym_handler = SymbolId(3);

        let main_symbol = make_symbol(
            sym_main,
            "main",
            SymbolKind::Function,
            root_mod,
            SymbolAttributes::empty(),
        );

        let mut test_attrs = SymbolAttributes::empty();
        test_attrs.is_test = true;
        let test_symbol = make_symbol(
            sym_test,
            "test_foo",
            SymbolKind::Function,
            root_mod,
            test_attrs,
        );

        let mut handler_attrs = SymbolAttributes::empty();
        handler_attrs.attribute_paths = vec![r#"get("/health")"#.to_owned()];
        let handler_symbol = make_symbol(
            sym_handler,
            "health_check",
            SymbolKind::Function,
            root_mod,
            handler_attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_main, main_symbol);
        symbols.insert(sym_test, test_symbol);
        symbols.insert(sym_handler, handler_symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(entrypoints.contains(&sym_main), "main should be entrypoint");
        assert!(entrypoints.contains(&sym_test), "test should be entrypoint");
        assert!(entrypoints.contains(&sym_handler), "handler should be entrypoint");
    }

    // -----------------------------------------------------------------------
    // Test: put and delete handler attributes
    // -----------------------------------------------------------------------

    #[test]
    fn put_and_delete_handler_attributes_detected() {
        let root_mod = ModuleId(0);
        let sym_put = SymbolId(1);
        let sym_delete = SymbolId(2);

        let mut put_attrs = SymbolAttributes::empty();
        put_attrs.attribute_paths = vec![r#"put("/api/item")"#.to_owned()];
        let put_symbol = make_symbol(
            sym_put,
            "update_item",
            SymbolKind::Function,
            root_mod,
            put_attrs,
        );

        let mut delete_attrs = SymbolAttributes::empty();
        delete_attrs.attribute_paths = vec![r#"delete("/api/item")"#.to_owned()];
        let delete_symbol = make_symbol(
            sym_delete,
            "delete_item",
            SymbolKind::Function,
            root_mod,
            delete_attrs,
        );

        let mut symbols = HashMap::new();
        symbols.insert(sym_put, put_symbol);
        symbols.insert(sym_delete, delete_symbol);

        let mut modules = HashMap::new();
        modules.insert(root_mod, make_root_module(root_mod));

        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);

        assert!(entrypoints.contains(&sym_put), "put handler should be entrypoint");
        assert!(entrypoints.contains(&sym_delete), "delete handler should be entrypoint");
    }

    // -----------------------------------------------------------------------
    // Test: empty symbol set produces no entrypoints
    // -----------------------------------------------------------------------

    #[test]
    fn empty_symbols_produce_no_entrypoints() {
        let symbols = HashMap::new();
        let modules = HashMap::new();
        let call_data = empty_call_graph_data();
        let entrypoints = detect_entrypoints(&symbols, &modules, &call_data);
        assert!(entrypoints.is_empty());
    }

    // -----------------------------------------------------------------------
    // Internal helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn has_any_attribute_matches() {
        let paths = vec!["tokio::main".to_owned(), "derive".to_owned()];
        assert!(has_any_attribute(&paths, &["tokio::main"]));
        assert!(!has_any_attribute(&paths, &["async_std::main"]));
    }

    #[test]
    fn has_any_attribute_prefix_matches() {
        let paths_with_args = vec![r#"get("/foo")"#.to_owned()];
        assert!(has_any_attribute_prefix(&paths_with_args, &["get"]));
        assert!(!has_any_attribute_prefix(&paths_with_args, &["post"]));

        let paths_bare = vec!["handler".to_owned()];
        assert!(has_any_attribute_prefix(&paths_bare, &["handler"]));
    }
}
