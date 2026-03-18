//! Module tree construction via lightweight text scanning.
//!
//! Starting from crate root files (`src/lib.rs` or `src/main.rs`), this module
//! scans for `mod` declarations using simple line-by-line text analysis (not
//! full AST parsing). It builds a [`ModuleInfo`] tree with parent/child
//! relationships.
//!
//! ## Detection rules
//!
//! For each line in a source file:
//!
//! 1. Lines inside block comments (`/* ... */`) are skipped.
//! 2. Lines starting with `//` (after stripping whitespace) are skipped.
//! 3. Lines starting with `#[path = "..."]` are recognized as path attributes
//!    and their value is carried forward to the next `mod` declaration.
//! 4. A line matching `mod <name>;` (with optional `pub`/`pub(...)` prefix)
//!    is an **external** module declaration. If a `#[path = "..."]` attribute
//!    preceded it, the custom path is used for file resolution; otherwise the
//!    standard `<name>.rs` or `<name>/mod.rs` convention applies.
//! 5. A line matching `mod <name> {` (with optional `pub`/`pub(...)` prefix)
//!    is an **inline** module declaration, recorded as a child module at the
//!    same file path.
//!
//! ## Edge cases
//!
//! - If the file for an external `mod` declaration does not exist, the module
//!   is recorded with `file_path: None` and a warning is logged.
//! - A `#[path = "..."]` attribute only applies to the immediately following
//!   `mod` declaration. If a non-attribute, non-mod line intervenes, the
//!   pending path is discarded.
//! - Non-UTF8 files are skipped with a warning.

use std::path::{Path, PathBuf};

use spectron_core::id::{IdGenerator, ModuleId};
use spectron_core::{CrateInfo, CrateType, ModuleInfo, ModulePath};

/// Discover the module tree for a single crate target.
///
/// Returns a flat list of [`ModuleInfo`] entries with parent/child
/// relationships set. The first entry is always the crate root module.
///
/// The `crate_info.module_ids` field is **not** modified by this function;
/// callers are responsible for collecting the returned module IDs.
pub fn discover_modules(id_gen: &IdGenerator, crate_info: &CrateInfo) -> Vec<ModuleInfo> {
    let root_file = match crate_root_file(crate_info) {
        Some(path) if path.exists() => path,
        _ => {
            tracing::debug!(
                crate_name = %crate_info.name,
                crate_type = ?crate_info.crate_type,
                "no root file found for crate, skipping module discovery"
            );
            return Vec::new();
        }
    };

    // The crate root module name: normalize hyphens to underscores (Rust convention).
    let crate_module_name = crate_info.name.replace('-', "_");

    let root_id = id_gen.next_module();
    let root_path = ModulePath::new(&crate_module_name);

    let mut modules = Vec::new();
    let mut root_module = ModuleInfo::new(
        root_id,
        &crate_module_name,
        root_path,
        Some(root_file.clone()),
        None,
    );

    // Scan the root file for child modules.
    let children = scan_file_for_modules(
        id_gen,
        &root_file,
        root_id,
        &crate_module_name,
        &mut modules,
    );
    root_module.children = children;

    // Insert root at the beginning.
    modules.insert(0, root_module);

    modules
}

/// Determine the crate root file path for a given crate target.
fn crate_root_file(crate_info: &CrateInfo) -> Option<PathBuf> {
    let src = crate_info.path.join("src");
    match crate_info.crate_type {
        CrateType::Library => Some(src.join("lib.rs")),
        CrateType::Binary => Some(src.join("main.rs")),
    }
}

/// Scan a source file for `mod` declarations and recursively discover child
/// modules.
///
/// Returns the list of child [`ModuleId`]s found in this file. All discovered
/// [`ModuleInfo`] entries (children, grandchildren, etc.) are appended to
/// `out_modules`.
fn scan_file_for_modules(
    id_gen: &IdGenerator,
    file_path: &Path,
    parent_id: ModuleId,
    parent_module_path: &str,
    out_modules: &mut Vec<ModuleInfo>,
) -> Vec<ModuleId> {
    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                path = %file_path.display(),
                error = %e,
                "failed to read source file for module scanning"
            );
            return Vec::new();
        }
    };

    let declarations = parse_mod_declarations(&content);
    let child_dir = module_child_dir(file_path);

    let mut child_ids = Vec::new();

    for decl in declarations {
        let child_id = id_gen.next_module();
        let child_path_str = format!("{}::{}", parent_module_path, decl.name);
        let child_path = ModulePath::new(&child_path_str);

        match decl.kind {
            ModDeclKind::External => {
                // Resolve to a file. If a #[path = "..."] attribute was
                // present, use that path relative to the child directory;
                // otherwise use the standard <name>.rs / <name>/mod.rs
                // convention.
                let resolved = if let Some(ref custom) = decl.custom_path {
                    resolve_custom_module_path(&child_dir, custom)
                } else {
                    resolve_external_module(&child_dir, &decl.name)
                };

                let mut child_module = ModuleInfo::new(
                    child_id,
                    &decl.name,
                    child_path,
                    resolved.clone(),
                    Some(parent_id),
                );

                if let Some(ref resolved_path) = resolved {
                    // Recurse into the resolved file.
                    let grandchildren = scan_file_for_modules(
                        id_gen,
                        resolved_path,
                        child_id,
                        &child_path_str,
                        out_modules,
                    );
                    child_module.children = grandchildren;
                } else {
                    tracing::warn!(
                        module_name = %decl.name,
                        parent_file = %file_path.display(),
                        "mod declaration found but no corresponding file exists"
                    );
                }

                child_ids.push(child_id);
                out_modules.push(child_module);
            }
            ModDeclKind::Inline => {
                // Inline module: same file as parent.
                let child_module = ModuleInfo::new(
                    child_id,
                    &decl.name,
                    child_path,
                    Some(file_path.to_path_buf()),
                    Some(parent_id),
                );
                // We do not recurse into inline modules since we cannot
                // reliably determine their boundaries with line scanning.
                child_ids.push(child_id);
                out_modules.push(child_module);
            }
        }
    }

    child_ids
}

/// Determine the directory in which child modules of a file should be resolved.
///
/// In Rust's module system:
/// - If the file is `mod.rs` or a crate root (`lib.rs`/`main.rs`), child
///   modules live in the same directory as the file.
/// - If the file is a regular `<name>.rs` file, child modules live in a
///   sibling directory named `<name>/`.
fn module_child_dir(file_path: &Path) -> PathBuf {
    let file_name = file_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    let parent_dir = file_path.parent().unwrap_or(Path::new("."));

    if file_name == "mod.rs" || file_name == "lib.rs" || file_name == "main.rs" {
        parent_dir.to_path_buf()
    } else {
        // For `src/foo.rs`, children are in `src/foo/`.
        let stem = file_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        parent_dir.join(stem)
    }
}

/// Resolve an external module name to a file path.
///
/// Checks for `<name>.rs` first, then `<name>/mod.rs`. Returns `None` if
/// neither exists.
fn resolve_external_module(parent_dir: &Path, module_name: &str) -> Option<PathBuf> {
    // Check <name>.rs
    let file_path = parent_dir.join(format!("{}.rs", module_name));
    if file_path.exists() {
        return Some(file_path);
    }

    // Check <name>/mod.rs
    let mod_path = parent_dir.join(module_name).join("mod.rs");
    if mod_path.exists() {
        return Some(mod_path);
    }

    None
}

/// Resolve a module using a custom `#[path = "..."]` attribute value.
///
/// The path is resolved relative to the parent directory (the directory where
/// child modules of the containing file are looked up). Returns `None` if the
/// resolved path does not exist on disk.
fn resolve_custom_module_path(parent_dir: &Path, custom_path: &str) -> Option<PathBuf> {
    let resolved = parent_dir.join(custom_path);
    if resolved.exists() {
        Some(resolved)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Mod declaration parsing
// ---------------------------------------------------------------------------

/// The kind of a `mod` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ModDeclKind {
    /// `mod foo;` -- references an external file.
    External,
    /// `mod foo { ... }` -- inline module body.
    Inline,
}

/// A parsed `mod` declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModDecl {
    /// The module name (e.g. `"foo"` from `mod foo;`).
    name: String,
    /// Whether this is an external or inline declaration.
    kind: ModDeclKind,
    /// Optional custom path from a `#[path = "..."]` attribute.
    ///
    /// When present, the external module file is resolved relative to the
    /// parent directory using this path instead of the default `<name>.rs` /
    /// `<name>/mod.rs` convention.
    custom_path: Option<String>,
}

/// Parse all `mod` declarations from source text using lightweight line scanning.
///
/// This function handles:
/// - Line comments (`//`)
/// - Block comments (`/* ... */`), including multi-line
/// - `pub`, `pub(crate)`, `pub(super)`, `pub(in ...)` visibility prefixes
/// - `mod name;` (external)
/// - `mod name { ... }` (inline)
/// - `#[path = "custom.rs"]` attributes on mod declarations
///
/// It does NOT handle:
/// - `mod` inside string literals (could produce false positives, but rare)
/// - Conditional compilation (`#[cfg(...)]` on mod items)
fn parse_mod_declarations(source: &str) -> Vec<ModDecl> {
    let mut declarations = Vec::new();
    let mut in_block_comment = false;
    // Tracks a pending `#[path = "..."]` value seen on a preceding attribute
    // line. Consumed by the next `mod` declaration and then reset to `None`.
    let mut pending_path_attr: Option<String> = None;

    for line in source.lines() {
        let trimmed = line.trim();

        // Handle block comment state.
        if in_block_comment {
            if let Some(_pos) = trimmed.find("*/") {
                in_block_comment = false;
                // There could be code after the block comment close on the
                // same line, but for simplicity we skip the entire line.
            }
            continue;
        }

        // Check for block comment start.
        if trimmed.starts_with("/*") {
            if !trimmed.contains("*/") {
                in_block_comment = true;
            }
            continue;
        }

        // Skip line comments.
        if trimmed.starts_with("//") {
            continue;
        }

        // Check for attributes. We specifically look for `#[path = "..."]`
        // and record the value for the next mod declaration.
        if trimmed.starts_with('#') {
            if let Some(path_value) = try_parse_path_attribute(trimmed) {
                pending_path_attr = Some(path_value);
            }
            // Whether or not this was a path attribute, skip the line
            // (it is not a mod declaration itself).
            continue;
        }

        // Try to extract a mod declaration from this line.
        if let Some(mut decl) = try_parse_mod_line(trimmed) {
            decl.custom_path = pending_path_attr.take();
            declarations.push(decl);
        } else {
            // A non-attribute, non-mod line resets any pending path attribute.
            // The `#[path = ...]` only applies to the immediately following
            // item, so if something else intervenes, discard it.
            pending_path_attr = None;
        }
    }

    declarations
}

/// Try to extract the path string from a `#[path = "..."]` attribute line.
///
/// Returns `Some(path_string)` if this line matches the pattern, `None`
/// otherwise. Handles optional whitespace around `=` and both `"` delimiters.
fn try_parse_path_attribute(line: &str) -> Option<String> {
    // We expect something like: #[path = "some/path.rs"]
    // Possibly with inner whitespace variations.
    let inner = line.strip_prefix("#[")?;
    let inner = inner.strip_suffix(']')?;
    let inner = inner.trim();

    // Must start with "path"
    let rest = inner.strip_prefix("path")?;
    let rest = rest.trim_start();

    // Must have "="
    let rest = rest.strip_prefix('=')?;
    let rest = rest.trim_start();

    // Must have a quoted string
    let rest = rest.strip_prefix('"')?;
    let end_quote = rest.find('"')?;
    let path_value = &rest[..end_quote];

    if path_value.is_empty() {
        return None;
    }

    Some(path_value.to_string())
}

/// Try to parse a single line as a `mod` declaration.
///
/// Accepts lines like:
/// - `mod foo;`
/// - `pub mod foo;`
/// - `pub(crate) mod foo;`
/// - `pub(super) mod foo;`
/// - `pub(in crate::path) mod foo;`
/// - `mod foo {`
/// - `pub mod foo {`
///
/// Returns `None` if the line is not a mod declaration.
fn try_parse_mod_line(line: &str) -> Option<ModDecl> {
    let rest = strip_visibility_prefix(line);

    // After stripping visibility, the line should start with "mod ".
    let rest = rest.strip_prefix("mod ")?;
    let rest = rest.trim_start();

    // The next token should be a valid Rust identifier.
    let (name, remainder) = take_identifier(rest)?;

    // Check what follows the identifier.
    let remainder = remainder.trim_start();

    if remainder.starts_with(';') {
        // External module: `mod foo;`
        Some(ModDecl {
            name: name.to_string(),
            kind: ModDeclKind::External,
            custom_path: None,
        })
    } else if remainder.starts_with('{') {
        // Inline module: `mod foo {`
        Some(ModDecl {
            name: name.to_string(),
            kind: ModDeclKind::Inline,
            custom_path: None,
        })
    } else {
        None
    }
}

/// Strip an optional visibility prefix from the beginning of a line.
///
/// Handles: `pub`, `pub(crate)`, `pub(super)`, `pub(self)`,
/// `pub(in some::path)`.
///
/// Returns the remaining string after the visibility prefix (or the
/// original string if no prefix is found).
fn strip_visibility_prefix(line: &str) -> &str {
    if !line.starts_with("pub") {
        return line;
    }

    let after_pub = &line[3..];

    // Check for `pub(...)` form.
    let after_pub_trimmed = after_pub.trim_start();
    if after_pub_trimmed.starts_with('(') {
        // Find the matching closing paren.
        if let Some(close) = after_pub_trimmed.find(')') {
            let after_paren = &after_pub_trimmed[close + 1..];
            return after_paren.trim_start();
        }
        // Malformed -- fall through and return after "pub".
    }

    // Plain `pub` -- must be followed by whitespace.
    if after_pub.starts_with(' ') || after_pub.starts_with('\t') {
        return after_pub.trim_start();
    }

    // Not a valid visibility prefix (e.g. `publish`).
    line
}

/// Extract a Rust identifier from the start of a string.
///
/// Returns `(identifier, rest)` or `None` if the string does not start with
/// a valid identifier character.
fn take_identifier(s: &str) -> Option<(&str, &str)> {
    let mut chars = s.char_indices();

    // First character must be alphabetic or underscore.
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return None,
    }

    // Subsequent characters: alphanumeric or underscore.
    let end = loop {
        match chars.next() {
            Some((i, c)) if c.is_ascii_alphanumeric() || c == '_' => continue,
            Some((i, _)) => break i,
            None => break s.len(),
        }
    };

    Some((&s[..end], &s[end..]))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use spectron_core::IdGenerator;
    use std::fs;

    // =======================================================================
    // Unit tests for parse_mod_declarations
    // =======================================================================

    // -----------------------------------------------------------------------
    // Task-requested unit tests: mod scanning basics
    // -----------------------------------------------------------------------

    #[test]
    fn scan_two_external_mods_foo_and_bar() {
        // Scan source with `mod foo;` and `mod bar;`, verify two child modules found.
        let source = "mod foo;\nmod bar;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 2, "expected exactly 2 child modules");
        assert_eq!(decls[0].name, "foo");
        assert_eq!(decls[0].kind, ModDeclKind::External);
        assert_eq!(decls[1].name, "bar");
        assert_eq!(decls[1].kind, ModDeclKind::External);
    }

    #[test]
    fn scan_inline_mod_tests_detected() {
        // Scan source with inline `mod tests { ... }`, verify inline module detected.
        let source = r#"pub fn do_work() {}

mod tests {
    #[test]
    fn it_works() {
        assert!(true);
    }
}
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1, "expected exactly 1 inline module");
        assert_eq!(decls[0].name, "tests");
        assert_eq!(decls[0].kind, ModDeclKind::Inline);
    }

    #[test]
    fn scan_missing_module_file_graceful_degradation() {
        // Missing module file: verify graceful degradation.
        // Create a temp crate with `mod missing;` but no corresponding file.
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "mod missing;\n").unwrap();
        // Intentionally do NOT create missing.rs or missing/mod.rs.

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "graceful",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // Root module + the missing module = 2 entries.
        assert_eq!(modules.len(), 2, "module entry should still be created");

        let missing = modules.iter().find(|m| m.name == "missing").unwrap();
        assert!(
            missing.file_path.is_none(),
            "file_path should be None for a module whose file does not exist"
        );
        assert_eq!(missing.parent, Some(modules[0].id));
        assert_eq!(missing.path.as_str(), "graceful::missing");
        // The module should have no children since there is no file to scan.
        assert!(missing.children.is_empty());
    }

    // -----------------------------------------------------------------------
    // Original unit tests
    // -----------------------------------------------------------------------

    #[test]
    fn parse_simple_external_mod() {
        let source = "mod foo;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "foo");
        assert_eq!(decls[0].kind, ModDeclKind::External);
    }

    #[test]
    fn parse_pub_external_mod() {
        let source = "pub mod bar;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "bar");
        assert_eq!(decls[0].kind, ModDeclKind::External);
    }

    #[test]
    fn parse_pub_crate_mod() {
        let source = "pub(crate) mod internal;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "internal");
        assert_eq!(decls[0].kind, ModDeclKind::External);
    }

    #[test]
    fn parse_pub_super_mod() {
        let source = "pub(super) mod sibling;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "sibling");
        assert_eq!(decls[0].kind, ModDeclKind::External);
    }

    #[test]
    fn parse_pub_in_path_mod() {
        let source = "pub(in crate::some::path) mod deep;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "deep");
        assert_eq!(decls[0].kind, ModDeclKind::External);
    }

    #[test]
    fn parse_inline_mod() {
        let source = "mod tests {\n    #[test]\n    fn it_works() {}\n}\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "tests");
        assert_eq!(decls[0].kind, ModDeclKind::Inline);
    }

    #[test]
    fn parse_pub_inline_mod() {
        let source = "pub mod inner {\n    pub fn f() {}\n}\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "inner");
        assert_eq!(decls[0].kind, ModDeclKind::Inline);
    }

    #[test]
    fn parse_multiple_declarations() {
        let source = r#"mod alpha;
pub mod beta;
mod gamma {
    fn g() {}
}
pub(crate) mod delta;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 4);
        assert_eq!(decls[0].name, "alpha");
        assert_eq!(decls[0].kind, ModDeclKind::External);
        assert_eq!(decls[1].name, "beta");
        assert_eq!(decls[1].kind, ModDeclKind::External);
        assert_eq!(decls[2].name, "gamma");
        assert_eq!(decls[2].kind, ModDeclKind::Inline);
        assert_eq!(decls[3].name, "delta");
        assert_eq!(decls[3].kind, ModDeclKind::External);
    }

    #[test]
    fn skips_line_comments() {
        let source = r#"// mod commented_out;
mod real;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "real");
    }

    #[test]
    fn skips_block_comments() {
        let source = r#"/* mod commented_out; */
mod real;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "real");
    }

    #[test]
    fn skips_multiline_block_comments() {
        let source = r#"/*
mod commented_out;
mod also_commented;
*/
mod real;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "real");
    }

    #[test]
    fn skips_attributes() {
        let source = r#"#[cfg(test)]
mod tests;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "tests");
    }

    #[test]
    fn ignores_non_mod_lines() {
        let source = r#"use std::path::Path;

pub fn hello() {}

struct Foo;

mod real;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "real");
    }

    #[test]
    fn ignores_mod_in_function_context() {
        // This is a known limitation: we may pick up mod declarations inside
        // function bodies. For the loader's purposes this is acceptable since
        // mod declarations in function bodies are rare in practice and the
        // worst case is an extra module entry that does not resolve to a file.
        let source = r#"mod real;

fn foo() {
    // This is not a real mod declaration but line scanning may pick it up.
}
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "real");
    }

    #[test]
    fn handles_underscore_module_name() {
        let source = "mod _private;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "_private");
        assert_eq!(decls[0].kind, ModDeclKind::External);
    }

    #[test]
    fn handles_module_name_with_numbers() {
        let source = "mod handler_v2;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "handler_v2");
    }

    #[test]
    fn empty_source_produces_no_declarations() {
        let decls = parse_mod_declarations("");
        assert!(decls.is_empty());
    }

    #[test]
    fn no_mod_declarations_in_source() {
        let source = r#"use std::io;

pub fn main() {
    println!("hello");
}
"#;
        let decls = parse_mod_declarations(source);
        assert!(decls.is_empty());
    }

    #[test]
    fn does_not_match_module_keyword_in_use() {
        // "use mod_utils;" should not match.
        let source = "use mod_utils;\n";
        let decls = parse_mod_declarations(source);
        assert!(decls.is_empty());
    }

    #[test]
    fn mod_with_leading_whitespace() {
        let source = "    mod indented;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "indented");
    }

    #[test]
    fn pub_self_visibility() {
        let source = "pub(self) mod private_mod;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "private_mod");
    }

    // =======================================================================
    // Unit tests for strip_visibility_prefix
    // =======================================================================

    #[test]
    fn strip_no_prefix() {
        assert_eq!(strip_visibility_prefix("mod foo;"), "mod foo;");
    }

    #[test]
    fn strip_pub_prefix() {
        assert_eq!(strip_visibility_prefix("pub mod foo;"), "mod foo;");
    }

    #[test]
    fn strip_pub_crate_prefix() {
        assert_eq!(strip_visibility_prefix("pub(crate) mod foo;"), "mod foo;");
    }

    #[test]
    fn strip_pub_super_prefix() {
        assert_eq!(strip_visibility_prefix("pub(super) mod foo;"), "mod foo;");
    }

    #[test]
    fn strip_pub_in_path_prefix() {
        assert_eq!(
            strip_visibility_prefix("pub(in crate::path) mod foo;"),
            "mod foo;"
        );
    }

    #[test]
    fn strip_does_not_match_publish() {
        // "publish" starts with "pub" but is not a visibility prefix.
        assert_eq!(strip_visibility_prefix("publish mod foo;"), "publish mod foo;");
    }

    // =======================================================================
    // Unit tests for take_identifier
    // =======================================================================

    #[test]
    fn take_simple_identifier() {
        let (name, rest) = take_identifier("foo;").unwrap();
        assert_eq!(name, "foo");
        assert_eq!(rest, ";");
    }

    #[test]
    fn take_identifier_with_underscore() {
        let (name, rest) = take_identifier("my_mod {").unwrap();
        assert_eq!(name, "my_mod");
        assert_eq!(rest, " {");
    }

    #[test]
    fn take_identifier_starting_with_underscore() {
        let (name, rest) = take_identifier("_private;").unwrap();
        assert_eq!(name, "_private");
        assert_eq!(rest, ";");
    }

    #[test]
    fn take_identifier_with_numbers() {
        let (name, rest) = take_identifier("v2_handler;").unwrap();
        assert_eq!(name, "v2_handler");
        assert_eq!(rest, ";");
    }

    #[test]
    fn take_identifier_fails_on_number_start() {
        assert!(take_identifier("2bad").is_none());
    }

    #[test]
    fn take_identifier_fails_on_empty() {
        assert!(take_identifier("").is_none());
    }

    // =======================================================================
    // Unit tests for try_parse_mod_line
    // =======================================================================

    #[test]
    fn try_parse_external() {
        let decl = try_parse_mod_line("mod foo;").unwrap();
        assert_eq!(decl.name, "foo");
        assert_eq!(decl.kind, ModDeclKind::External);
    }

    #[test]
    fn try_parse_inline() {
        let decl = try_parse_mod_line("mod foo {").unwrap();
        assert_eq!(decl.name, "foo");
        assert_eq!(decl.kind, ModDeclKind::Inline);
    }

    #[test]
    fn try_parse_not_mod() {
        assert!(try_parse_mod_line("use foo;").is_none());
        assert!(try_parse_mod_line("fn main() {}").is_none());
        assert!(try_parse_mod_line("let x = 5;").is_none());
    }

    #[test]
    fn try_parse_pub_crate_inline() {
        let decl = try_parse_mod_line("pub(crate) mod inner {").unwrap();
        assert_eq!(decl.name, "inner");
        assert_eq!(decl.kind, ModDeclKind::Inline);
    }

    // =======================================================================
    // Unit tests for module_child_dir
    // =======================================================================

    #[test]
    fn module_child_dir_for_lib_rs() {
        let dir = module_child_dir(Path::new("/project/src/lib.rs"));
        assert_eq!(dir, PathBuf::from("/project/src"));
    }

    #[test]
    fn module_child_dir_for_main_rs() {
        let dir = module_child_dir(Path::new("/project/src/main.rs"));
        assert_eq!(dir, PathBuf::from("/project/src"));
    }

    #[test]
    fn module_child_dir_for_mod_rs() {
        let dir = module_child_dir(Path::new("/project/src/network/mod.rs"));
        assert_eq!(dir, PathBuf::from("/project/src/network"));
    }

    #[test]
    fn module_child_dir_for_regular_file() {
        let dir = module_child_dir(Path::new("/project/src/foo.rs"));
        assert_eq!(dir, PathBuf::from("/project/src/foo"));
    }

    #[test]
    fn module_child_dir_for_nested_regular_file() {
        let dir = module_child_dir(Path::new("/project/src/parent/child.rs"));
        assert_eq!(dir, PathBuf::from("/project/src/parent/child"));
    }

    // =======================================================================
    // Unit tests for resolve_external_module
    // =======================================================================

    #[test]
    fn resolve_to_file_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        fs::write(dir.join("foo.rs"), "// foo").unwrap();

        let result = resolve_external_module(dir, "foo");
        assert_eq!(result, Some(dir.join("foo.rs")));
    }

    #[test]
    fn resolve_to_mod_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        fs::create_dir_all(dir.join("bar")).unwrap();
        fs::write(dir.join("bar/mod.rs"), "// bar").unwrap();

        let result = resolve_external_module(dir, "bar");
        assert_eq!(result, Some(dir.join("bar/mod.rs")));
    }

    #[test]
    fn resolve_prefers_file_rs_over_mod_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        // Both foo.rs and foo/mod.rs exist; foo.rs should win.
        fs::write(dir.join("foo.rs"), "// foo file").unwrap();
        fs::create_dir_all(dir.join("foo")).unwrap();
        fs::write(dir.join("foo/mod.rs"), "// foo mod").unwrap();

        let result = resolve_external_module(dir, "foo");
        assert_eq!(result, Some(dir.join("foo.rs")));
    }

    #[test]
    fn resolve_returns_none_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = resolve_external_module(tmp.path(), "nonexistent");
        assert!(result.is_none());
    }

    // =======================================================================
    // Unit tests for crate_root_file
    // =======================================================================

    #[test]
    fn crate_root_file_library() {
        let id_gen = IdGenerator::new();
        let info = CrateInfo::new(
            id_gen.next_crate(),
            "mylib",
            "/tmp/mylib",
            CrateType::Library,
        );
        let root = crate_root_file(&info).unwrap();
        assert_eq!(root, PathBuf::from("/tmp/mylib/src/lib.rs"));
    }

    #[test]
    fn crate_root_file_binary() {
        let id_gen = IdGenerator::new();
        let info = CrateInfo::new(
            id_gen.next_crate(),
            "mybin",
            "/tmp/mybin",
            CrateType::Binary,
        );
        let root = crate_root_file(&info).unwrap();
        assert_eq!(root, PathBuf::from("/tmp/mybin/src/main.rs"));
    }

    // =======================================================================
    // Integration-style tests for discover_modules
    // =======================================================================

    #[test]
    fn discover_modules_simple_crate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("lib.rs"),
            "mod foo;\nmod bar;\n\npub fn root() {}\n",
        )
        .unwrap();
        fs::write(src.join("foo.rs"), "pub fn foo_fn() {}\n").unwrap();
        fs::write(src.join("bar.rs"), "pub fn bar_fn() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "my-crate",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // Should have root + foo + bar = 3 modules.
        assert_eq!(modules.len(), 3, "expected 3 modules, got {:?}", modules);

        // Root module.
        let root_mod = &modules[0];
        assert_eq!(root_mod.name, "my_crate");
        assert_eq!(root_mod.path.as_str(), "my_crate");
        assert!(root_mod.parent.is_none());
        assert_eq!(root_mod.children.len(), 2);
        assert!(root_mod.file_path.is_some());

        // Children should be foo and bar.
        let child_names: Vec<&str> = modules[1..].iter().map(|m| m.name.as_str()).collect();
        assert!(child_names.contains(&"foo"), "missing foo in {:?}", child_names);
        assert!(child_names.contains(&"bar"), "missing bar in {:?}", child_names);

        // Each child should have the root as parent.
        for child in &modules[1..] {
            assert_eq!(child.parent, Some(root_mod.id));
            assert!(child.file_path.is_some());
        }

        // Verify qualified paths.
        let foo = modules.iter().find(|m| m.name == "foo").unwrap();
        assert_eq!(foo.path.as_str(), "my_crate::foo");

        let bar = modules.iter().find(|m| m.name == "bar").unwrap();
        assert_eq!(bar.path.as_str(), "my_crate::bar");
    }

    #[test]
    fn discover_modules_nested_hierarchy() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("parent")).unwrap();

        fs::write(src.join("lib.rs"), "mod parent;\n").unwrap();
        fs::write(src.join("parent.rs"), "mod child;\n").unwrap();
        fs::write(src.join("parent/child.rs"), "pub fn leaf() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "nested",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root -> parent -> child = 3 modules.
        assert_eq!(modules.len(), 3, "expected 3 modules, got {:?}", modules);

        let root_mod = &modules[0];
        assert_eq!(root_mod.name, "nested");
        assert_eq!(root_mod.children.len(), 1);

        let parent_mod = modules.iter().find(|m| m.name == "parent").unwrap();
        assert_eq!(parent_mod.path.as_str(), "nested::parent");
        assert_eq!(parent_mod.parent, Some(root_mod.id));
        assert_eq!(parent_mod.children.len(), 1);

        let child_mod = modules.iter().find(|m| m.name == "child").unwrap();
        assert_eq!(child_mod.path.as_str(), "nested::parent::child");
        assert_eq!(child_mod.parent, Some(parent_mod.id));
        assert!(child_mod.children.is_empty());
    }

    #[test]
    fn discover_modules_with_mod_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("sub")).unwrap();

        fs::write(src.join("lib.rs"), "mod sub;\n").unwrap();
        fs::write(src.join("sub/mod.rs"), "pub fn sub_fn() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "modrs-crate",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        assert_eq!(modules.len(), 2);
        let sub_mod = modules.iter().find(|m| m.name == "sub").unwrap();
        assert!(sub_mod.file_path.as_ref().unwrap().ends_with("sub/mod.rs"));
    }

    #[test]
    fn discover_modules_inline_module() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("lib.rs"),
            r#"mod tests {
    fn it_works() {}
}
"#,
        )
        .unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "inline-mod",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        assert_eq!(modules.len(), 2, "expected root + inline = 2, got {:?}", modules);

        let tests_mod = modules.iter().find(|m| m.name == "tests").unwrap();
        // Inline module shares the parent file path.
        assert_eq!(
            tests_mod.file_path.as_ref().unwrap(),
            modules[0].file_path.as_ref().unwrap(),
            "inline module should share the parent file path"
        );
        assert_eq!(tests_mod.parent, Some(modules[0].id));
    }

    #[test]
    fn discover_modules_missing_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(src.join("lib.rs"), "mod nonexistent;\n").unwrap();
        // Do NOT create nonexistent.rs or nonexistent/mod.rs.

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "missing-file",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        assert_eq!(modules.len(), 2, "should still create module entry");

        let missing = modules.iter().find(|m| m.name == "nonexistent").unwrap();
        assert!(
            missing.file_path.is_none(),
            "file_path should be None for missing module file"
        );
        assert_eq!(missing.parent, Some(modules[0].id));
    }

    #[test]
    fn discover_modules_binary_crate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("main.rs"),
            "mod config;\n\nfn main() {}\n",
        )
        .unwrap();
        fs::write(src.join("config.rs"), "pub static PORT: u16 = 8080;\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "my-app",
            root,
            CrateType::Binary,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        assert_eq!(modules.len(), 2);
        assert_eq!(modules[0].name, "my_app");
        assert!(modules[0].file_path.as_ref().unwrap().ends_with("main.rs"));

        let config = modules.iter().find(|m| m.name == "config").unwrap();
        assert_eq!(config.path.as_str(), "my_app::config");
    }

    #[test]
    fn discover_modules_empty_crate() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(src.join("lib.rs"), "// empty crate\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "empty",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // Just the root module.
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "empty");
        assert!(modules[0].children.is_empty());
    }

    #[test]
    fn discover_modules_no_src_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // No src/ directory.

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "no-src",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);
        assert!(modules.is_empty());
    }

    #[test]
    fn discover_modules_unique_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("lib.rs"),
            "mod a;\nmod b;\nmod c;\n",
        )
        .unwrap();
        fs::write(src.join("a.rs"), "").unwrap();
        fs::write(src.join("b.rs"), "").unwrap();
        fs::write(src.join("c.rs"), "").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "ids-test",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);
        assert_eq!(modules.len(), 4);

        let ids: std::collections::HashSet<_> = modules.iter().map(|m| m.id).collect();
        assert_eq!(ids.len(), 4, "all module IDs should be unique");
    }

    #[test]
    fn discover_modules_deeply_nested() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("a/b")).unwrap();

        fs::write(src.join("lib.rs"), "mod a;\n").unwrap();
        fs::write(src.join("a.rs"), "mod b;\n").unwrap();
        fs::write(src.join("a/b.rs"), "mod c;\n").unwrap();
        // c does not exist -- should record with file_path: None.

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "deep",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root -> a -> b -> c(missing) = 4 modules
        assert_eq!(modules.len(), 4, "expected 4 modules, got {:?}", modules);

        let c = modules.iter().find(|m| m.name == "c").unwrap();
        assert_eq!(c.path.as_str(), "deep::a::b::c");
        assert!(c.file_path.is_none());
    }

    #[test]
    fn discover_modules_mixed_external_and_inline() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        fs::write(
            src.join("lib.rs"),
            r#"mod external;

pub mod inline {
    pub fn inline_fn() {}
}
"#,
        )
        .unwrap();
        fs::write(src.join("external.rs"), "pub fn ext_fn() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "mixed",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root + external + inline = 3
        assert_eq!(modules.len(), 3);

        let ext = modules.iter().find(|m| m.name == "external").unwrap();
        assert!(ext.file_path.is_some());
        assert_eq!(ext.path.as_str(), "mixed::external");

        let inl = modules.iter().find(|m| m.name == "inline").unwrap();
        assert!(inl.file_path.is_some());
        assert_eq!(inl.path.as_str(), "mixed::inline");
    }

    #[test]
    fn discover_modules_with_nested_mod_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("network")).unwrap();

        fs::write(src.join("lib.rs"), "mod network;\n").unwrap();
        fs::write(src.join("network/mod.rs"), "mod tcp;\n").unwrap();
        fs::write(src.join("network/tcp.rs"), "pub fn connect() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "netlib",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root -> network -> tcp = 3
        assert_eq!(modules.len(), 3);

        let network = modules.iter().find(|m| m.name == "network").unwrap();
        assert_eq!(network.path.as_str(), "netlib::network");
        assert_eq!(network.children.len(), 1);

        let tcp = modules.iter().find(|m| m.name == "tcp").unwrap();
        assert_eq!(tcp.path.as_str(), "netlib::network::tcp");
        assert_eq!(tcp.parent, Some(network.id));
    }

    // =======================================================================
    // Unit tests for try_parse_path_attribute
    // =======================================================================

    #[test]
    fn parse_path_attr_basic() {
        let result = try_parse_path_attribute(r#"#[path = "custom.rs"]"#);
        assert_eq!(result, Some("custom.rs".to_string()));
    }

    #[test]
    fn parse_path_attr_with_directory() {
        let result = try_parse_path_attribute(r#"#[path = "platform/linux.rs"]"#);
        assert_eq!(result, Some("platform/linux.rs".to_string()));
    }

    #[test]
    fn parse_path_attr_no_spaces_around_eq() {
        let result = try_parse_path_attribute(r#"#[path="custom.rs"]"#);
        assert_eq!(result, Some("custom.rs".to_string()));
    }

    #[test]
    fn parse_path_attr_extra_spaces() {
        let result = try_parse_path_attribute(r#"#[path  =  "custom.rs"]"#);
        assert_eq!(result, Some("custom.rs".to_string()));
    }

    #[test]
    fn parse_path_attr_empty_value_returns_none() {
        let result = try_parse_path_attribute(r#"#[path = ""]"#);
        assert!(result.is_none());
    }

    #[test]
    fn parse_path_attr_not_a_path_attr() {
        assert!(try_parse_path_attribute(r#"#[cfg(test)]"#).is_none());
        assert!(try_parse_path_attribute(r#"#[derive(Debug)]"#).is_none());
        assert!(try_parse_path_attribute(r#"#[allow(unused)]"#).is_none());
    }

    #[test]
    fn parse_path_attr_missing_closing_bracket() {
        let result = try_parse_path_attribute(r#"#[path = "custom.rs""#);
        assert!(result.is_none());
    }

    #[test]
    fn parse_path_attr_missing_opening_bracket() {
        let result = try_parse_path_attribute(r#"path = "custom.rs"]"#);
        assert!(result.is_none());
    }

    // =======================================================================
    // Unit tests for resolve_custom_module_path
    // =======================================================================

    #[test]
    fn resolve_custom_path_to_existing_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        fs::write(dir.join("custom_impl.rs"), "// custom").unwrap();

        let result = resolve_custom_module_path(dir, "custom_impl.rs");
        assert_eq!(result, Some(dir.join("custom_impl.rs")));
    }

    #[test]
    fn resolve_custom_path_to_subdirectory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dir = tmp.path();
        fs::create_dir_all(dir.join("platform")).unwrap();
        fs::write(dir.join("platform/linux.rs"), "// linux").unwrap();

        let result = resolve_custom_module_path(dir, "platform/linux.rs");
        assert_eq!(result, Some(dir.join("platform/linux.rs")));
    }

    #[test]
    fn resolve_custom_path_returns_none_when_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let result = resolve_custom_module_path(tmp.path(), "nonexistent.rs");
        assert!(result.is_none());
    }

    // =======================================================================
    // Unit tests for parse_mod_declarations with #[path = "..."]
    // =======================================================================

    #[test]
    fn parse_path_attr_on_external_mod() {
        let source = r#"#[path = "custom_impl.rs"]
mod foo;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "foo");
        assert_eq!(decls[0].kind, ModDeclKind::External);
        assert_eq!(
            decls[0].custom_path.as_deref(),
            Some("custom_impl.rs")
        );
    }

    #[test]
    fn parse_path_attr_on_pub_external_mod() {
        let source = r#"#[path = "os_specific.rs"]
pub mod platform;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "platform");
        assert_eq!(decls[0].kind, ModDeclKind::External);
        assert_eq!(
            decls[0].custom_path.as_deref(),
            Some("os_specific.rs")
        );
    }

    #[test]
    fn parse_path_attr_on_inline_mod() {
        let source = r#"#[path = "custom_dir"]
mod foo {
}
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "foo");
        assert_eq!(decls[0].kind, ModDeclKind::Inline);
        assert_eq!(
            decls[0].custom_path.as_deref(),
            Some("custom_dir")
        );
    }

    #[test]
    fn parse_path_attr_with_other_attrs_before() {
        // #[cfg(test)] then #[path = "..."] then mod -- the path attribute
        // should still be captured because non-path attributes don't reset
        // the pending state.
        let source = r#"#[cfg(test)]
#[path = "test_impl.rs"]
mod tests;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "tests");
        assert_eq!(
            decls[0].custom_path.as_deref(),
            Some("test_impl.rs")
        );
    }

    #[test]
    fn parse_path_attr_with_other_attrs_after() {
        // #[path = "..."] then #[cfg(test)] then mod -- the path attribute
        // should still be carried since the second attribute doesn't replace it
        // (it is not a path attr), and attributes don't reset pending state.
        let source = r#"#[path = "test_impl.rs"]
#[cfg(target_os = "linux")]
mod platform;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "platform");
        assert_eq!(
            decls[0].custom_path.as_deref(),
            Some("test_impl.rs")
        );
    }

    #[test]
    fn parse_path_attr_reset_by_intervening_code() {
        // A non-attribute, non-mod line between #[path] and mod should reset
        // the pending path attribute.
        let source = r#"#[path = "custom.rs"]
use std::io;
mod foo;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "foo");
        assert!(
            decls[0].custom_path.is_none(),
            "path attr should be reset by intervening use statement"
        );
    }

    #[test]
    fn parse_no_path_attr_produces_none() {
        let source = "mod foo;\n";
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 1);
        assert!(
            decls[0].custom_path.is_none(),
            "mod without #[path] should have custom_path = None"
        );
    }

    #[test]
    fn parse_path_attr_only_applies_to_next_mod() {
        // The path attribute should only apply to the immediately following mod,
        // not to subsequent mods.
        let source = r#"#[path = "custom.rs"]
mod first;
mod second;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "first");
        assert_eq!(
            decls[0].custom_path.as_deref(),
            Some("custom.rs")
        );
        assert_eq!(decls[1].name, "second");
        assert!(
            decls[1].custom_path.is_none(),
            "second mod should not inherit the path attribute"
        );
    }

    #[test]
    fn parse_multiple_path_attrs_on_separate_mods() {
        let source = r#"#[path = "impl_a.rs"]
mod a;
#[path = "impl_b.rs"]
mod b;
mod c;
"#;
        let decls = parse_mod_declarations(source);
        assert_eq!(decls.len(), 3);
        assert_eq!(decls[0].custom_path.as_deref(), Some("impl_a.rs"));
        assert_eq!(decls[1].custom_path.as_deref(), Some("impl_b.rs"));
        assert!(decls[2].custom_path.is_none());
    }

    // =======================================================================
    // Integration tests for discover_modules with #[path = "..."]
    // =======================================================================

    #[test]
    fn discover_modules_with_path_attr_custom_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        // lib.rs uses #[path = "impl_foo.rs"] mod foo;
        // The actual file is impl_foo.rs, not foo.rs.
        fs::write(
            src.join("lib.rs"),
            "#[path = \"impl_foo.rs\"]\nmod foo;\n",
        )
        .unwrap();
        fs::write(src.join("impl_foo.rs"), "pub fn custom() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "path-attr",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root + foo = 2 modules
        assert_eq!(modules.len(), 2, "expected 2 modules, got {:?}", modules);

        let foo = modules.iter().find(|m| m.name == "foo").unwrap();
        assert_eq!(foo.path.as_str(), "path_attr::foo");
        assert!(foo.file_path.is_some(), "foo should resolve to impl_foo.rs");
        assert!(
            foo.file_path.as_ref().unwrap().ends_with("impl_foo.rs"),
            "expected impl_foo.rs, got {:?}",
            foo.file_path
        );
    }

    #[test]
    fn discover_modules_with_path_attr_subdirectory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("platform")).unwrap();

        // lib.rs uses #[path = "platform/linux.rs"] mod os;
        fs::write(
            src.join("lib.rs"),
            "#[path = \"platform/linux.rs\"]\nmod os;\n",
        )
        .unwrap();
        fs::write(
            src.join("platform/linux.rs"),
            "pub fn init() {}\n",
        )
        .unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "platform-crate",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        assert_eq!(modules.len(), 2);

        let os_mod = modules.iter().find(|m| m.name == "os").unwrap();
        assert!(os_mod.file_path.is_some());
        assert!(
            os_mod
                .file_path
                .as_ref()
                .unwrap()
                .ends_with("platform/linux.rs")
                || os_mod
                    .file_path
                    .as_ref()
                    .unwrap()
                    .ends_with("platform\\linux.rs"),
            "expected platform/linux.rs, got {:?}",
            os_mod.file_path
        );
    }

    #[test]
    fn discover_modules_with_path_attr_missing_custom_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        // lib.rs uses #[path = "nonexistent.rs"] mod foo;
        // The custom file does not exist.
        fs::write(
            src.join("lib.rs"),
            "#[path = \"nonexistent.rs\"]\nmod foo;\n",
        )
        .unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "missing-custom",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        assert_eq!(modules.len(), 2, "should still create module entry");

        let foo = modules.iter().find(|m| m.name == "foo").unwrap();
        assert!(
            foo.file_path.is_none(),
            "file_path should be None when custom path does not exist"
        );
    }

    #[test]
    fn discover_modules_path_attr_with_recursion() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(src.join("impls")).unwrap();

        // lib.rs:  #[path = "impls/real_foo.rs"] mod foo;
        // impls/real_foo.rs: mod bar;
        // impls/real_foo/bar.rs: (leaf)
        //
        // The child module `bar` of the custom-path file should resolve
        // relative to its own location (impls/real_foo/).
        fs::create_dir_all(src.join("impls/real_foo")).unwrap();
        fs::write(
            src.join("lib.rs"),
            "#[path = \"impls/real_foo.rs\"]\nmod foo;\n",
        )
        .unwrap();
        fs::write(
            src.join("impls/real_foo.rs"),
            "mod bar;\n",
        )
        .unwrap();
        fs::write(
            src.join("impls/real_foo/bar.rs"),
            "pub fn leaf() {}\n",
        )
        .unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "recursive-path",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root -> foo -> bar = 3 modules
        assert_eq!(modules.len(), 3, "expected 3 modules, got {:?}", modules);

        let foo = modules.iter().find(|m| m.name == "foo").unwrap();
        assert!(foo.file_path.is_some());
        assert_eq!(foo.children.len(), 1);

        let bar = modules.iter().find(|m| m.name == "bar").unwrap();
        assert_eq!(bar.path.as_str(), "recursive_path::foo::bar");
        assert!(bar.file_path.is_some());
        assert_eq!(bar.parent, Some(foo.id));
    }

    #[test]
    fn discover_modules_path_attr_mixed_with_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let src = root.join("src");
        fs::create_dir_all(&src).unwrap();

        // lib.rs has one module with #[path] and one without.
        fs::write(
            src.join("lib.rs"),
            r#"#[path = "custom_alpha.rs"]
mod alpha;
mod beta;
"#,
        )
        .unwrap();
        fs::write(src.join("custom_alpha.rs"), "pub fn a() {}\n").unwrap();
        fs::write(src.join("beta.rs"), "pub fn b() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let crate_info = CrateInfo::new(
            id_gen.next_crate(),
            "mixed-paths",
            root,
            CrateType::Library,
        );

        let modules = discover_modules(&id_gen, &crate_info);

        // root + alpha + beta = 3
        assert_eq!(modules.len(), 3);

        let alpha = modules.iter().find(|m| m.name == "alpha").unwrap();
        assert!(alpha.file_path.is_some());
        assert!(
            alpha
                .file_path
                .as_ref()
                .unwrap()
                .ends_with("custom_alpha.rs"),
            "alpha should resolve to custom_alpha.rs"
        );

        let beta = modules.iter().find(|m| m.name == "beta").unwrap();
        assert!(beta.file_path.is_some());
        assert!(
            beta.file_path.as_ref().unwrap().ends_with("beta.rs"),
            "beta should resolve to beta.rs via standard convention"
        );
    }
}
