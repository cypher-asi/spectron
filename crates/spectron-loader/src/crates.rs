//! Crate type detection and enumeration.
//!
//! For each discovered crate directory, determines what targets exist
//! (library, binary, or both) by inspecting the filesystem layout and
//! `[[bin]]` sections in `Cargo.toml`.

use std::path::Path;

use spectron_core::id::IdGenerator;
use spectron_core::{CrateInfo, CrateType};

/// Discover crate targets (library and/or binary) for a single crate directory.
///
/// Detection logic:
/// 1. If `src/lib.rs` exists, register a [`CrateType::Library`] target.
/// 2. If `src/main.rs` exists, register a [`CrateType::Binary`] target.
/// 3. Parse `[[bin]]` sections in `Cargo.toml` for additional binary targets
///    beyond the default `src/main.rs`.
/// 4. If neither lib.rs, main.rs, nor any `[[bin]]` targets are found, fall
///    back to registering a Library target with a warning.
///
/// When `[[bin]]` entries exist, any entry whose path resolves to `src/main.rs`
/// is skipped if a binary was already registered from step 2, to avoid
/// double-counting.
pub fn discover_crate_targets(
    id_gen: &IdGenerator,
    crate_name: &str,
    crate_path: &Path,
) -> Vec<CrateInfo> {
    let mut targets = Vec::new();
    let src_dir = crate_path.join("src");

    // Extract dependency names from Cargo.toml once -- shared across all
    // targets of this crate since they all come from the same manifest.
    let dependencies = crate::manifest::extract_dependency_names(
        &crate_path.join("Cargo.toml"),
    );

    // Step 1: Check for library target.
    if src_dir.join("lib.rs").exists() {
        let id = id_gen.next_crate();
        let mut info = CrateInfo::new(id, crate_name, crate_path, CrateType::Library);
        info.dependencies = dependencies.clone();
        targets.push(info);
    }

    // Step 2: Check for default binary target (src/main.rs).
    let has_default_binary = src_dir.join("main.rs").exists();
    if has_default_binary {
        let id = id_gen.next_crate();
        let mut info = CrateInfo::new(id, crate_name, crate_path, CrateType::Binary);
        info.dependencies = dependencies.clone();
        targets.push(info);
    }

    // Step 3: Check [[bin]] sections for additional binary targets.
    let additional_bins = discover_bin_targets(crate_name, crate_path, has_default_binary);
    for bin_name in additional_bins {
        let id = id_gen.next_crate();
        let mut info = CrateInfo::new(id, &bin_name, crate_path, CrateType::Binary);
        info.dependencies = dependencies.clone();
        targets.push(info);
    }

    // Step 4: Fallback -- if no targets found, register as Library with a warning.
    if targets.is_empty() {
        tracing::warn!(
            crate_name = crate_name,
            path = %crate_path.display(),
            "no src/lib.rs or src/main.rs found for crate"
        );
        let id = id_gen.next_crate();
        let mut info = CrateInfo::new(id, crate_name, crate_path, CrateType::Library);
        info.dependencies = dependencies;
        targets.push(info);
    }

    targets
}

/// Parse `[[bin]]` sections from the crate's `Cargo.toml` and return names
/// of additional binary targets that are NOT the default `src/main.rs`.
///
/// If `has_default_binary` is true, any `[[bin]]` entry whose path is
/// `src/main.rs` (or unset, which defaults to `src/main.rs`) is skipped
/// to avoid double-counting.
///
/// Returns an empty vec if the manifest cannot be parsed or has no `[[bin]]`
/// entries.
fn discover_bin_targets(
    crate_name: &str,
    crate_path: &Path,
    has_default_binary: bool,
) -> Vec<String> {
    let manifest_path = crate_path.join("Cargo.toml");
    let content = match std::fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                path = %manifest_path.display(),
                error = %e,
                "could not read Cargo.toml for [[bin]] detection"
            );
            return Vec::new();
        }
    };

    let manifest = match cargo_toml::Manifest::from_str(&content) {
        Ok(m) => m,
        Err(e) => {
            tracing::debug!(
                path = %manifest_path.display(),
                error = %e,
                "could not parse Cargo.toml for [[bin]] detection"
            );
            return Vec::new();
        }
    };

    extract_additional_bin_names(&manifest, crate_name, has_default_binary)
}

/// Extract additional binary target names from a parsed manifest.
///
/// This is separated from `discover_bin_targets` so it can be unit-tested
/// without needing a filesystem.
fn extract_additional_bin_names(
    manifest: &cargo_toml::Manifest,
    crate_name: &str,
    has_default_binary: bool,
) -> Vec<String> {
    let mut names = Vec::new();

    for bin in &manifest.bin {
        let bin_name = bin
            .name
            .as_deref()
            .unwrap_or(crate_name);

        let bin_path = bin
            .path
            .as_deref()
            .unwrap_or("src/main.rs");

        // Normalize the path for comparison.
        let is_default_main = is_default_main_path(bin_path);

        // Skip if this is the default src/main.rs binary and we already
        // registered it from filesystem detection.
        if has_default_binary && is_default_main {
            tracing::debug!(
                bin_name = bin_name,
                "skipping [[bin]] entry that points to default src/main.rs"
            );
            continue;
        }

        names.push(bin_name.to_string());
    }

    names
}

/// Check whether a binary target path refers to the default `src/main.rs`.
fn is_default_main_path(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized == "src/main.rs"
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;

    // -----------------------------------------------------------------------
    // Unit tests for discover_crate_targets
    // -----------------------------------------------------------------------

    #[test]
    fn library_only_when_only_lib_rs_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Create Cargo.toml and src/lib.rs only.
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "lib-only"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "lib-only", root);

        assert_eq!(targets.len(), 1, "expected 1 target, got {}", targets.len());
        assert_eq!(targets[0].crate_type, CrateType::Library);
        assert_eq!(targets[0].name, "lib-only");
        assert_eq!(targets[0].path, root);
    }

    #[test]
    fn binary_only_when_only_main_rs_exists() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "bin-only"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "bin-only", root);

        assert_eq!(targets.len(), 1, "expected 1 target, got {}", targets.len());
        assert_eq!(targets[0].crate_type, CrateType::Binary);
        assert_eq!(targets[0].name, "bin-only");
        assert_eq!(targets[0].path, root);
    }

    #[test]
    fn both_targets_when_lib_and_main_exist() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "dual"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "dual", root);

        assert_eq!(targets.len(), 2, "expected 2 targets, got {}", targets.len());

        let types: Vec<&CrateType> = targets.iter().map(|t| &t.crate_type).collect();
        assert!(types.contains(&&CrateType::Library), "expected Library target");
        assert!(types.contains(&&CrateType::Binary), "expected Binary target");

        // Both should have the same name and path.
        for t in &targets {
            assert_eq!(t.name, "dual");
            assert_eq!(t.path, root);
        }
    }

    #[test]
    fn crate_info_fields_populated_correctly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn hello() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "my-crate", root);

        assert_eq!(targets.len(), 1);
        let ci = &targets[0];

        // Verify all CrateInfo fields.
        assert_eq!(ci.name, "my-crate");
        assert_eq!(ci.path, root);
        assert_eq!(ci.crate_type, CrateType::Library);
        assert!(ci.module_ids.is_empty(), "module_ids should start empty");
    }

    #[test]
    fn unique_crate_ids_assigned() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "dual-ids"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "dual-ids", root);

        assert_eq!(targets.len(), 2);
        let ids: HashSet<_> = targets.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 2, "each target should have a unique CrateId");
    }

    #[test]
    fn fallback_to_library_when_no_source_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Cargo.toml with no src/ directory at all.
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "empty"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "empty", root);

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].crate_type, CrateType::Library);
        assert_eq!(targets[0].name, "empty");
    }

    // -----------------------------------------------------------------------
    // [[bin]] section tests
    // -----------------------------------------------------------------------

    #[test]
    fn bin_section_registers_additional_binary_target() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // A library crate with an additional [[bin]] target.
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "my-lib"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-tool"
path = "src/bin/tool.rs"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();
        fs::write(root.join("src/bin/tool.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "my-lib", root);

        // Should have: 1 Library (lib.rs) + 1 Binary (my-tool from [[bin]])
        assert_eq!(targets.len(), 2, "expected 2 targets, got {}", targets.len());

        let lib_targets: Vec<_> = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Library)
            .collect();
        let bin_targets: Vec<_> = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Binary)
            .collect();

        assert_eq!(lib_targets.len(), 1);
        assert_eq!(lib_targets[0].name, "my-lib");

        assert_eq!(bin_targets.len(), 1);
        assert_eq!(bin_targets[0].name, "my-tool");
    }

    #[test]
    fn bin_section_does_not_duplicate_default_main() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Has src/main.rs AND a [[bin]] entry pointing to src/main.rs.
        // Should NOT double-count the binary.
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "my-cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-cli"
path = "src/main.rs"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "my-cli", root);

        // Should have exactly 1 binary (not 2).
        assert_eq!(targets.len(), 1, "expected 1 target, got {}", targets.len());
        assert_eq!(targets[0].crate_type, CrateType::Binary);
    }

    #[test]
    fn multiple_bin_sections_register_multiple_binaries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "multi-bin"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "tool-a"
path = "src/bin/a.rs"

[[bin]]
name = "tool-b"
path = "src/bin/b.rs"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();
        fs::write(root.join("src/bin/a.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("src/bin/b.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "multi-bin", root);

        // Should have: 1 Library + 2 Binaries.
        assert_eq!(targets.len(), 3, "expected 3 targets, got {}", targets.len());

        let bin_names: Vec<&str> = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Binary)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(bin_names.len(), 2);
        assert!(bin_names.contains(&"tool-a"), "missing tool-a in {:?}", bin_names);
        assert!(bin_names.contains(&"tool-b"), "missing tool-b in {:?}", bin_names);
    }

    #[test]
    fn bin_section_with_main_and_additional_binaries() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // Has src/main.rs, src/lib.rs, and a [[bin]] with a non-default path.
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "combo"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "extra-tool"
path = "src/bin/extra.rs"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn lib_fn() {}\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("src/bin/extra.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "combo", root);

        // Should have: 1 Library (lib.rs) + 1 Binary (main.rs) + 1 Binary (extra-tool).
        assert_eq!(targets.len(), 3, "expected 3 targets, got {}", targets.len());

        let lib_count = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Library)
            .count();
        let bin_count = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Binary)
            .count();

        assert_eq!(lib_count, 1, "expected 1 library");
        assert_eq!(bin_count, 2, "expected 2 binaries");

        let bin_names: Vec<&str> = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Binary)
            .map(|t| t.name.as_str())
            .collect();
        assert!(bin_names.contains(&"combo"), "missing default binary 'combo'");
        assert!(bin_names.contains(&"extra-tool"), "missing extra-tool");
    }

    #[test]
    fn bin_section_name_defaults_to_crate_name() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        // [[bin]] without a name should default to the crate name.
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "nameless-bin"
version = "0.1.0"
edition = "2021"

[[bin]]
path = "src/bin/custom.rs"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(root.join("src/bin/custom.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "nameless-bin", root);

        // Should register 1 binary with the crate name.
        let bin_targets: Vec<_> = targets
            .iter()
            .filter(|t| t.crate_type == CrateType::Binary)
            .collect();
        assert_eq!(bin_targets.len(), 1);
        assert_eq!(bin_targets[0].name, "nameless-bin");
    }

    // -----------------------------------------------------------------------
    // Unit tests for extract_additional_bin_names (no filesystem needed)
    // -----------------------------------------------------------------------

    #[test]
    fn extract_no_bin_sections_returns_empty() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "no-bins"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        let names = extract_additional_bin_names(&manifest, "no-bins", false);
        assert!(names.is_empty());
    }

    #[test]
    fn extract_skips_default_main_when_has_default_binary() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "cli"
path = "src/main.rs"
"#,
        )
        .unwrap();

        let names = extract_additional_bin_names(&manifest, "cli", true);
        assert!(names.is_empty(), "should skip default main.rs bin, got: {:?}", names);
    }

    #[test]
    fn extract_includes_default_main_when_no_default_binary() {
        // If src/main.rs does not exist but [[bin]] points to it,
        // we should still include it (the file just was not detected by
        // the filesystem check, but the manifest declares it).
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "cli"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "cli"
path = "src/main.rs"
"#,
        )
        .unwrap();

        let names = extract_additional_bin_names(&manifest, "cli", false);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "cli");
    }

    #[test]
    fn extract_non_default_path_always_included() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "multi"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "tool"
path = "src/bin/tool.rs"
"#,
        )
        .unwrap();

        let names = extract_additional_bin_names(&manifest, "multi", true);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0], "tool");
    }

    // -----------------------------------------------------------------------
    // Unit test for is_default_main_path
    // -----------------------------------------------------------------------

    #[test]
    fn is_default_main_path_matches_forward_slash() {
        assert!(is_default_main_path("src/main.rs"));
    }

    #[test]
    fn is_default_main_path_matches_backslash() {
        assert!(is_default_main_path("src\\main.rs"));
    }

    #[test]
    fn is_default_main_path_rejects_other_paths() {
        assert!(!is_default_main_path("src/bin/tool.rs"));
        assert!(!is_default_main_path("src/other_main.rs"));
        assert!(!is_default_main_path("main.rs"));
    }

    #[test]
    fn library_target_listed_before_binary() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();

        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "order-test"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let targets = discover_crate_targets(&id_gen, "order-test", root);

        assert_eq!(targets.len(), 2);
        // Library should come first, then binary.
        assert_eq!(targets[0].crate_type, CrateType::Library);
        assert_eq!(targets[1].crate_type, CrateType::Binary);
    }
}
