//! Security indicator detection for the analysis engine.
//!
//! This module implements Section 7 of the analysis spec:
//! - Collect unsafe code (unsafe functions, unsafe blocks)
//! - Collect FFI boundaries (extern symbols)
//! - Detect sensitive API calls (filesystem, network, subprocess)
//!
//! All detected indicators are aggregated into a `SecurityReport`.

use std::collections::HashMap;

use spectron_core::{
    ModuleId, ModuleInfo, SecurityIndicator, SecurityReport,
    Symbol, SymbolId,
};
use spectron_graph::CallGraphData;

// ---------------------------------------------------------------------------
// Sensitive API prefix lists
// ---------------------------------------------------------------------------

/// Filesystem access path prefixes.
const FILESYSTEM_PREFIXES: &[&str] = &[
    "std::fs::",
    "tokio::fs::",
];

/// Network access path prefixes.
const NETWORK_PREFIXES: &[&str] = &[
    "std::net::",
    "tokio::net::",
    "reqwest::",
    "hyper::",
];

/// Subprocess execution path prefixes.
const SUBPROCESS_PREFIXES: &[&str] = &[
    "std::process::Command",
    "tokio::process::Command",
];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect all security indicators from the given symbols and call graph.
///
/// Walks all symbols to collect:
/// 1. `SecurityIndicator::UnsafeFunction` for symbols where `is_unsafe == true`
/// 2. `SecurityIndicator::UnsafeBlock` for symbols where `has_unsafe_block == true`
/// 3. `SecurityIndicator::FfiCall` for symbols where `is_extern == true`
///
/// Then walks the call graph to detect sensitive API calls:
/// 4. `SecurityIndicator::FilesystemAccess` for calls matching filesystem prefixes
/// 5. `SecurityIndicator::NetworkAccess` for calls matching network prefixes
/// 6. `SecurityIndicator::SubprocessExecution` for calls matching subprocess prefixes
pub fn detect_security_indicators(
    symbols: &HashMap<SymbolId, Symbol>,
    modules: &HashMap<ModuleId, ModuleInfo>,
    call_graph_data: &CallGraphData,
) -> SecurityReport {
    let mut report = SecurityReport::new();

    // Step 1-3: Walk all symbols for unsafe code and FFI boundaries.
    for (_sym_id, symbol) in symbols {
        // 1. Unsafe functions
        if symbol.attributes.is_unsafe {
            report.indicators.push(SecurityIndicator::UnsafeFunction {
                symbol_id: symbol.id,
            });
        }

        // 2. Unsafe blocks
        if symbol.attributes.has_unsafe_block {
            report.indicators.push(SecurityIndicator::UnsafeBlock {
                span: symbol.span.clone(),
            });
        }

        // 3. FFI boundaries
        if symbol.attributes.is_extern {
            report.indicators.push(SecurityIndicator::FfiCall {
                span: symbol.span.clone(),
                extern_name: symbol.name.clone(),
            });
        }
    }

    // Step 4-6: Walk call graph to detect sensitive API calls.
    //
    // For each caller, look at its callees. Resolve each callee to a
    // qualified name (module_path::symbol_name) and check against known
    // sensitive API prefixes.
    for (caller_id, callees) in &call_graph_data.callees {
        let caller_span = symbols
            .get(caller_id)
            .map(|s| s.span.clone());

        for callee_id in callees {
            let callee = match symbols.get(callee_id) {
                Some(s) => s,
                None => continue,
            };

            let qualified_name = resolve_qualified_name(callee, modules);

            // Use a default span if the caller is not in the symbol table.
            // This is defensive; in practice the caller should always exist.
            let span = caller_span
                .clone()
                .unwrap_or_else(|| callee.span.clone());

            if matches_any_prefix(&qualified_name, FILESYSTEM_PREFIXES) {
                report.indicators.push(SecurityIndicator::FilesystemAccess {
                    span,
                    function_name: qualified_name.clone(),
                });
            } else if matches_any_prefix(&qualified_name, NETWORK_PREFIXES) {
                report.indicators.push(SecurityIndicator::NetworkAccess {
                    span,
                    function_name: qualified_name.clone(),
                });
            } else if matches_any_prefix(&qualified_name, SUBPROCESS_PREFIXES) {
                report.indicators.push(SecurityIndicator::SubprocessExecution {
                    span,
                    function_name: qualified_name.clone(),
                });
            }
        }
    }

    report
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Resolve the fully qualified name for a symbol by combining its module
/// path with the symbol name.
///
/// If the module is found, returns `"module::path::symbol_name"`.
/// Otherwise, falls back to just the symbol name.
fn resolve_qualified_name(
    symbol: &Symbol,
    modules: &HashMap<ModuleId, ModuleInfo>,
) -> String {
    match modules.get(&symbol.module_id) {
        Some(module_info) => {
            format!("{}::{}", module_info.path.as_str(), symbol.name)
        }
        None => symbol.name.clone(),
    }
}

/// Check if a qualified name starts with any of the given prefixes.
fn matches_any_prefix(name: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| name.starts_with(prefix))
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
        module_id: ModuleId,
        attributes: SymbolAttributes,
    ) -> Symbol {
        let file_id = FileId(0);
        Symbol {
            id,
            name: name.to_owned(),
            kind: SymbolKind::Function,
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

    fn make_module(id: ModuleId, path: &str) -> ModuleInfo {
        ModuleInfo::new(
            id,
            path.rsplit("::").next().unwrap_or(path),
            ModulePath::new(path),
            None,
            None,
        )
    }

    // -----------------------------------------------------------------------
    // Test: Symbol with is_unsafe -> appears as UnsafeFunction
    // -----------------------------------------------------------------------

    #[test]
    fn unsafe_function_detected() {
        let mod_id = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.is_unsafe = true;

        let symbol = make_symbol(sym_id, "danger_fn", mod_id, attrs);

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let modules = HashMap::new();
        let call_data = empty_call_graph_data();

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let unsafe_fn_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::UnsafeFunction { symbol_id } if *symbol_id == sym_id))
            .count();

        assert_eq!(
            unsafe_fn_count, 1,
            "symbol with is_unsafe=true should produce exactly one UnsafeFunction indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Symbol with has_unsafe_block -> appears as UnsafeBlock
    // -----------------------------------------------------------------------

    #[test]
    fn unsafe_block_detected() {
        let mod_id = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.has_unsafe_block = true;

        let symbol = make_symbol(sym_id, "tricky_fn", mod_id, attrs);

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let modules = HashMap::new();
        let call_data = empty_call_graph_data();

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let unsafe_block_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::UnsafeBlock { .. }))
            .count();

        assert_eq!(
            unsafe_block_count, 1,
            "symbol with has_unsafe_block=true should produce exactly one UnsafeBlock indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Symbol with is_extern -> appears as FfiCall
    // -----------------------------------------------------------------------

    #[test]
    fn ffi_boundary_detected() {
        let mod_id = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.is_extern = true;

        let symbol = make_symbol(sym_id, "libc_call", mod_id, attrs);

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let modules = HashMap::new();
        let call_data = empty_call_graph_data();

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let ffi_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::FfiCall { extern_name, .. } if extern_name == "libc_call"))
            .count();

        assert_eq!(
            ffi_count, 1,
            "symbol with is_extern=true should produce exactly one FfiCall indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call to std::fs::write -> appears as FilesystemAccess
    // -----------------------------------------------------------------------

    #[test]
    fn filesystem_access_detected() {
        let mod_caller = ModuleId(0);
        let mod_fs = ModuleId(1);
        let sym_caller = SymbolId(10);
        let sym_write = SymbolId(20);

        let caller = make_symbol(sym_caller, "do_work", mod_caller, SymbolAttributes::empty());
        let write_fn = make_symbol(sym_write, "write", mod_fs, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_caller, caller);
        symbols.insert(sym_write, write_fn);

        let mut modules = HashMap::new();
        modules.insert(mod_caller, make_module(mod_caller, "my_crate"));
        modules.insert(mod_fs, make_module(mod_fs, "std::fs"));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_write, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_write]);
                m.insert(sym_write, vec![]);
                m
            },
        };

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let fs_count = report
            .indicators
            .iter()
            .filter(|i| matches!(
                i,
                SecurityIndicator::FilesystemAccess { function_name, .. }
                if function_name == "std::fs::write"
            ))
            .count();

        assert_eq!(
            fs_count, 1,
            "call to std::fs::write should produce exactly one FilesystemAccess indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call to std::net::TcpStream::connect -> appears as NetworkAccess
    // -----------------------------------------------------------------------

    #[test]
    fn network_access_detected() {
        let mod_caller = ModuleId(0);
        let mod_net = ModuleId(1);
        let sym_caller = SymbolId(10);
        let sym_connect = SymbolId(20);

        let caller = make_symbol(sym_caller, "fetch", mod_caller, SymbolAttributes::empty());
        // The symbol name is "connect" and it lives in module "std::net::TcpStream"
        // so qualified name becomes "std::net::TcpStream::connect"
        let connect_fn = make_symbol(sym_connect, "connect", mod_net, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_caller, caller);
        symbols.insert(sym_connect, connect_fn);

        let mut modules = HashMap::new();
        modules.insert(mod_caller, make_module(mod_caller, "my_crate"));
        modules.insert(mod_net, make_module(mod_net, "std::net::TcpStream"));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_connect, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_connect]);
                m.insert(sym_connect, vec![]);
                m
            },
        };

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let net_count = report
            .indicators
            .iter()
            .filter(|i| matches!(
                i,
                SecurityIndicator::NetworkAccess { function_name, .. }
                if function_name == "std::net::TcpStream::connect"
            ))
            .count();

        assert_eq!(
            net_count, 1,
            "call to std::net::TcpStream::connect should produce exactly one NetworkAccess indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call to std::process::Command::new -> appears as SubprocessExecution
    // -----------------------------------------------------------------------

    #[test]
    fn subprocess_execution_detected() {
        let mod_caller = ModuleId(0);
        let mod_process = ModuleId(1);
        let sym_caller = SymbolId(10);
        let sym_cmd = SymbolId(20);

        let caller = make_symbol(sym_caller, "run_cmd", mod_caller, SymbolAttributes::empty());
        let cmd_fn = make_symbol(sym_cmd, "new", mod_process, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_caller, caller);
        symbols.insert(sym_cmd, cmd_fn);

        let mut modules = HashMap::new();
        modules.insert(mod_caller, make_module(mod_caller, "my_crate"));
        modules.insert(mod_process, make_module(mod_process, "std::process::Command"));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_cmd, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_cmd]);
                m.insert(sym_cmd, vec![]);
                m
            },
        };

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let proc_count = report
            .indicators
            .iter()
            .filter(|i| matches!(
                i,
                SecurityIndicator::SubprocessExecution { function_name, .. }
                if function_name == "std::process::Command::new"
            ))
            .count();

        assert_eq!(
            proc_count, 1,
            "call to std::process::Command::new should produce one SubprocessExecution indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: tokio::fs call detected as FilesystemAccess
    // -----------------------------------------------------------------------

    #[test]
    fn tokio_fs_detected_as_filesystem_access() {
        let mod_caller = ModuleId(0);
        let mod_tokio_fs = ModuleId(1);
        let sym_caller = SymbolId(10);
        let sym_read = SymbolId(20);

        let caller = make_symbol(sym_caller, "load_data", mod_caller, SymbolAttributes::empty());
        let read_fn = make_symbol(sym_read, "read", mod_tokio_fs, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_caller, caller);
        symbols.insert(sym_read, read_fn);

        let mut modules = HashMap::new();
        modules.insert(mod_caller, make_module(mod_caller, "my_crate"));
        modules.insert(mod_tokio_fs, make_module(mod_tokio_fs, "tokio::fs"));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_read, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_read]);
                m.insert(sym_read, vec![]);
                m
            },
        };

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let fs_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::FilesystemAccess { .. }))
            .count();

        assert_eq!(
            fs_count, 1,
            "call to tokio::fs::read should produce a FilesystemAccess indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: reqwest and hyper calls detected as NetworkAccess
    // -----------------------------------------------------------------------

    #[test]
    fn reqwest_detected_as_network_access() {
        let mod_caller = ModuleId(0);
        let mod_reqwest = ModuleId(1);
        let sym_caller = SymbolId(10);
        let sym_get = SymbolId(20);

        let caller = make_symbol(sym_caller, "fetch_url", mod_caller, SymbolAttributes::empty());
        let get_fn = make_symbol(sym_get, "get", mod_reqwest, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_caller, caller);
        symbols.insert(sym_get, get_fn);

        let mut modules = HashMap::new();
        modules.insert(mod_caller, make_module(mod_caller, "my_crate"));
        modules.insert(mod_reqwest, make_module(mod_reqwest, "reqwest"));

        let call_data = CallGraphData {
            callers: {
                let mut m = HashMap::new();
                m.insert(sym_get, vec![sym_caller]);
                m.insert(sym_caller, vec![]);
                m
            },
            callees: {
                let mut m = HashMap::new();
                m.insert(sym_caller, vec![sym_get]);
                m.insert(sym_get, vec![]);
                m
            },
        };

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let net_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::NetworkAccess { .. }))
            .count();

        assert_eq!(
            net_count, 1,
            "call to reqwest::get should produce a NetworkAccess indicator"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Symbol with both is_unsafe and has_unsafe_block produces both
    // -----------------------------------------------------------------------

    #[test]
    fn both_unsafe_function_and_block_detected() {
        let mod_id = ModuleId(0);
        let sym_id = SymbolId(1);

        let mut attrs = SymbolAttributes::empty();
        attrs.is_unsafe = true;
        attrs.has_unsafe_block = true;

        let symbol = make_symbol(sym_id, "very_unsafe", mod_id, attrs);

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let modules = HashMap::new();
        let call_data = empty_call_graph_data();

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        let unsafe_fn_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::UnsafeFunction { .. }))
            .count();

        let unsafe_block_count = report
            .indicators
            .iter()
            .filter(|i| matches!(i, SecurityIndicator::UnsafeBlock { .. }))
            .count();

        assert_eq!(unsafe_fn_count, 1, "should have one UnsafeFunction indicator");
        assert_eq!(unsafe_block_count, 1, "should have one UnsafeBlock indicator");
    }

    // -----------------------------------------------------------------------
    // Test: Normal symbol produces no indicators
    // -----------------------------------------------------------------------

    #[test]
    fn normal_symbol_produces_no_indicators() {
        let mod_id = ModuleId(0);
        let sym_id = SymbolId(1);

        let symbol = make_symbol(sym_id, "safe_fn", mod_id, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_id, symbol);

        let modules = HashMap::new();
        let call_data = empty_call_graph_data();

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        assert!(
            report.indicators.is_empty(),
            "normal symbol should produce no security indicators"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Call to non-sensitive function produces no indicator
    // -----------------------------------------------------------------------

    #[test]
    fn non_sensitive_call_produces_no_indicator() {
        let mod_caller = ModuleId(0);
        let mod_callee = ModuleId(1);
        let sym_caller = SymbolId(10);
        let sym_callee = SymbolId(20);

        let caller = make_symbol(sym_caller, "main", mod_caller, SymbolAttributes::empty());
        let callee = make_symbol(sym_callee, "helper", mod_callee, SymbolAttributes::empty());

        let mut symbols = HashMap::new();
        symbols.insert(sym_caller, caller);
        symbols.insert(sym_callee, callee);

        let mut modules = HashMap::new();
        modules.insert(mod_caller, make_module(mod_caller, "my_crate"));
        modules.insert(mod_callee, make_module(mod_callee, "my_crate::utils"));

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

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        assert!(
            report.indicators.is_empty(),
            "call to non-sensitive function should produce no indicators"
        );
    }

    // -----------------------------------------------------------------------
    // Test: Empty input produces empty report
    // -----------------------------------------------------------------------

    #[test]
    fn empty_input_produces_empty_report() {
        let symbols = HashMap::new();
        let modules = HashMap::new();
        let call_data = empty_call_graph_data();

        let report = detect_security_indicators(&symbols, &modules, &call_data);

        assert!(report.indicators.is_empty());
    }

    // -----------------------------------------------------------------------
    // Internal helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn matches_any_prefix_works() {
        assert!(matches_any_prefix("std::fs::write", FILESYSTEM_PREFIXES));
        assert!(matches_any_prefix("std::fs::read_to_string", FILESYSTEM_PREFIXES));
        assert!(matches_any_prefix("tokio::fs::read", FILESYSTEM_PREFIXES));
        assert!(!matches_any_prefix("std::io::read", FILESYSTEM_PREFIXES));

        assert!(matches_any_prefix("std::net::TcpStream::connect", NETWORK_PREFIXES));
        assert!(matches_any_prefix("tokio::net::TcpListener::bind", NETWORK_PREFIXES));
        assert!(matches_any_prefix("reqwest::get", NETWORK_PREFIXES));
        assert!(matches_any_prefix("hyper::Client::new", NETWORK_PREFIXES));
        assert!(!matches_any_prefix("my_crate::network::send", NETWORK_PREFIXES));

        assert!(matches_any_prefix("std::process::Command::new", SUBPROCESS_PREFIXES));
        assert!(matches_any_prefix("tokio::process::Command::spawn", SUBPROCESS_PREFIXES));
        assert!(!matches_any_prefix("std::process::exit", SUBPROCESS_PREFIXES));
    }

    #[test]
    fn resolve_qualified_name_with_module() {
        let mod_id = ModuleId(0);
        let sym = make_symbol(SymbolId(1), "write", mod_id, SymbolAttributes::empty());

        let mut modules = HashMap::new();
        modules.insert(mod_id, make_module(mod_id, "std::fs"));

        let name = resolve_qualified_name(&sym, &modules);
        assert_eq!(name, "std::fs::write");
    }

    #[test]
    fn resolve_qualified_name_without_module() {
        let mod_id = ModuleId(999);
        let sym = make_symbol(SymbolId(1), "write", mod_id, SymbolAttributes::empty());

        let modules = HashMap::new();

        let name = resolve_qualified_name(&sym, &modules);
        assert_eq!(name, "write", "should fall back to symbol name when module not found");
    }
}
