//! Integration tests for `spectron-loader`'s `load_project` function.
//!
//! These tests use on-disk test fixtures located under `tests/fixtures/` to
//! exercise the full project-loading pipeline end-to-end.

use std::path::{Path, PathBuf};

use spectron_core::error::SpectronError;
use spectron_core::{CrateType, ModuleInfo};
use spectron_loader::load_project;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Return the absolute path to a fixture directory.
///
/// Fixtures live under `<crate-root>/tests/fixtures/<name>`.
fn fixture_path(name: &str) -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join("tests").join("fixtures").join(name)
}

// ---------------------------------------------------------------------------
// Single-crate fixture
// ---------------------------------------------------------------------------

#[test]
fn load_single_crate_project() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed for single_crate");

    // Project metadata
    assert_eq!(result.project.name, "my-single-crate");
    assert!(!result.project.is_workspace);

    // Exactly one library crate target
    assert_eq!(
        result.crates.len(),
        1,
        "expected 1 crate target, got {}",
        result.crates.len()
    );
    let krate = &result.crates[0];
    assert_eq!(krate.name, "my-single-crate");
    assert_eq!(krate.crate_type, CrateType::Library);

    // Project crate_ids should reference the discovered crate
    assert_eq!(result.project.crate_ids.len(), 1);
    assert_eq!(result.project.crate_ids[0], krate.id);
}

#[test]
fn load_single_crate_with_both_lib_and_bin() {
    let path = fixture_path("single_crate_both");
    let result =
        load_project(&path).expect("load_project should succeed for single_crate_both");

    assert_eq!(result.project.name, "dual-target");
    assert!(!result.project.is_workspace);

    // Should discover two targets: library + binary
    assert_eq!(
        result.crates.len(),
        2,
        "expected 2 crate targets (lib+bin), got {}",
        result.crates.len()
    );

    let types: Vec<&CrateType> = result.crates.iter().map(|c| &c.crate_type).collect();
    assert!(
        types.contains(&&CrateType::Library),
        "expected a Library target"
    );
    assert!(
        types.contains(&&CrateType::Binary),
        "expected a Binary target"
    );

    // All crate IDs should be tracked in the project
    assert_eq!(result.project.crate_ids.len(), 2);
}

// ---------------------------------------------------------------------------
// Workspace fixture
// ---------------------------------------------------------------------------

#[test]
fn load_workspace_project() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed for workspace");

    // Project metadata
    assert!(
        result.project.is_workspace,
        "workspace fixture should be detected as a workspace"
    );

    // The workspace name is derived from the directory name (no [package] section).
    assert_eq!(result.project.name, "workspace");

    // Two member crates: alpha (lib) and beta (bin)
    assert!(
        result.crates.len() >= 2,
        "expected at least 2 crate targets, got {}",
        result.crates.len()
    );

    let crate_names: Vec<&str> = result.crates.iter().map(|c| c.name.as_str()).collect();
    assert!(
        crate_names.contains(&"alpha"),
        "expected crate 'alpha', found: {:?}",
        crate_names
    );
    assert!(
        crate_names.contains(&"beta"),
        "expected crate 'beta', found: {:?}",
        crate_names
    );

    // Verify crate types
    let alpha = result.crates.iter().find(|c| c.name == "alpha").unwrap();
    assert_eq!(
        alpha.crate_type,
        CrateType::Library,
        "alpha should be a library"
    );

    let beta = result.crates.iter().find(|c| c.name == "beta").unwrap();
    assert_eq!(
        beta.crate_type,
        CrateType::Binary,
        "beta should be a binary"
    );

    // Project should track all discovered crate IDs
    assert!(
        result.project.crate_ids.len() >= 2,
        "project should reference at least 2 crate IDs"
    );
}

// ---------------------------------------------------------------------------
// No Cargo.toml
// ---------------------------------------------------------------------------

#[test]
fn load_project_returns_no_cargo_for_missing_manifest() {
    let path = fixture_path("nonexistent_directory_that_does_not_exist");
    let err = load_project(&path).expect_err("should return error for missing directory");

    match err {
        SpectronError::NoCargo { path: err_path } => {
            assert_eq!(err_path, path);
        }
        other => panic!("expected NoCargo, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Malformed fixtures
// ---------------------------------------------------------------------------

#[test]
fn load_project_returns_error_for_bad_toml() {
    let path = fixture_path("malformed/bad_toml");
    let err = load_project(&path).expect_err("should return error for bad TOML");

    match err {
        SpectronError::Parse { file, message } => {
            assert!(
                file.to_string_lossy().contains("Cargo.toml"),
                "error file should reference Cargo.toml, got: {:?}",
                file
            );
            assert!(
                !message.is_empty(),
                "parse error message should not be empty"
            );
        }
        other => panic!("expected Parse error, got: {:?}", other),
    }
}

#[test]
fn load_project_returns_error_for_no_package_section() {
    let path = fixture_path("malformed/no_package");
    let err = load_project(&path)
        .expect_err("should return error for Cargo.toml without [package] or [workspace]");

    match err {
        SpectronError::Parse { message, .. } => {
            assert!(
                message.contains("neither [workspace] nor [package]"),
                "unexpected error message: {}",
                message
            );
        }
        other => panic!("expected Parse error, got: {:?}", other),
    }
}

#[test]
fn load_workspace_with_missing_member_skips_gracefully() {
    let path = fixture_path("malformed/missing_member");
    let result = load_project(&path)
        .expect("load_project should succeed even with a missing workspace member");

    assert!(result.project.is_workspace);

    // Only the "exists" crate should be discovered; the "missing" member is skipped.
    let crate_names: Vec<&str> = result.crates.iter().map(|c| c.name.as_str()).collect();
    assert!(
        crate_names.contains(&"exists"),
        "expected crate 'exists', found: {:?}",
        crate_names
    );

    // The missing member should NOT appear as a crate.
    // The total count should be 1 (just the "exists" library).
    assert_eq!(
        result.crates.len(),
        1,
        "expected exactly 1 crate (missing member skipped), got {}: {:?}",
        result.crates.len(),
        crate_names
    );
}

#[test]
fn load_project_with_missing_module_file_succeeds() {
    // The missing_module fixture has `mod nonexistent;` in lib.rs but no
    // corresponding file. The loader should still succeed loading the project
    // structure (crate discovery). Module tree construction records the module
    // with file_path: None.
    let path = fixture_path("malformed/missing_module");
    let result = load_project(&path)
        .expect("load_project should succeed despite a missing module file");

    assert_eq!(result.project.name, "missing-module-crate");
    assert!(!result.project.is_workspace);
    assert_eq!(result.crates.len(), 1);
    assert_eq!(result.crates[0].crate_type, CrateType::Library);
}

// ---------------------------------------------------------------------------
// Edge case: empty src directory
// ---------------------------------------------------------------------------

#[test]
fn load_project_with_no_source_files() {
    // Use tempfile to create a crate directory with a Cargo.toml but no src/
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let cargo_toml = tmp.path().join("Cargo.toml");
    std::fs::write(
        &cargo_toml,
        r#"[package]
name = "empty-crate"
version = "0.1.0"
edition = "2021"
"#,
    )
    .expect("failed to write Cargo.toml");

    let result = load_project(tmp.path()).expect("should succeed for crate with no src/");

    assert_eq!(result.project.name, "empty-crate");
    assert!(!result.project.is_workspace);

    // The loader should still register the crate (with a warning about missing source).
    assert_eq!(
        result.crates.len(),
        1,
        "should still register the crate even with no source files"
    );
}

// ---------------------------------------------------------------------------
// Temporary directory tests (not relying on fixtures)
// ---------------------------------------------------------------------------

#[test]
fn load_project_from_temp_single_crate() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    // Write Cargo.toml
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "temp-single"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    // Write src/lib.rs
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "pub fn temp() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");

    assert_eq!(result.project.name, "temp-single");
    assert!(!result.project.is_workspace);
    assert_eq!(result.crates.len(), 1);
    assert_eq!(result.crates[0].name, "temp-single");
    assert_eq!(result.crates[0].crate_type, CrateType::Library);
}

#[test]
fn load_project_from_temp_workspace() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    // Write root Cargo.toml
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["lib-a", "lib-b"]
resolver = "2"
"#,
    )
    .unwrap();

    // lib-a: library
    let lib_a = root.join("lib-a");
    std::fs::create_dir_all(lib_a.join("src")).unwrap();
    std::fs::write(
        lib_a.join("Cargo.toml"),
        r#"[package]
name = "lib-a"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::write(lib_a.join("src/lib.rs"), "pub fn a() {}\n").unwrap();

    // lib-b: binary
    let lib_b = root.join("lib-b");
    std::fs::create_dir_all(lib_b.join("src")).unwrap();
    std::fs::write(
        lib_b.join("Cargo.toml"),
        r#"[package]
name = "lib-b"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::write(lib_b.join("src/main.rs"), "fn main() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");

    assert!(result.project.is_workspace);
    assert!(
        result.crates.len() >= 2,
        "expected at least 2 crates, got {}",
        result.crates.len()
    );

    let names: Vec<&str> = result.crates.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"lib-a"), "missing lib-a in {:?}", names);
    assert!(names.contains(&"lib-b"), "missing lib-b in {:?}", names);
}

// ---------------------------------------------------------------------------
// File discovery integration tests
// ---------------------------------------------------------------------------

#[test]
fn load_single_crate_discovers_all_rs_files() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    // single_crate fixture has: src/lib.rs, src/models.rs, src/utils.rs
    assert_eq!(
        result.files.len(),
        3,
        "expected 3 .rs files in single_crate, got {}",
        result.files.len()
    );

    // Every file should have a valid SHA-256 hash (64 hex chars).
    for f in &result.files {
        assert_eq!(
            f.hash.len(),
            64,
            "SHA-256 hex hash should be 64 chars, got {} for {:?}",
            f.hash.len(),
            f.path
        );
        // Hash should be lowercase hex.
        assert!(
            f.hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hash should be hex, got: {}",
            f.hash
        );
    }

    // Every file should have a positive line count (none of our fixtures are empty).
    for f in &result.files {
        assert!(
            f.line_count > 0,
            "line count should be > 0 for {:?}",
            f.path
        );
    }

    // Verify all paths end in .rs.
    for f in &result.files {
        assert!(
            f.path.extension().and_then(|e| e.to_str()) == Some("rs"),
            "expected .rs extension, got {:?}",
            f.path
        );
    }

    // All file IDs should be unique.
    let ids: std::collections::HashSet<_> = result.files.iter().map(|f| f.id).collect();
    assert_eq!(ids.len(), result.files.len(), "file IDs should be unique");
}

#[test]
fn load_single_crate_both_does_not_duplicate_files() {
    // single_crate_both has both lib.rs and main.rs, so two crate targets
    // share the same src/ directory. Files should not be duplicated.
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    // single_crate_both has: src/lib.rs, src/main.rs, src/shared.rs
    assert_eq!(
        result.files.len(),
        3,
        "expected 3 .rs files in single_crate_both (no duplicates), got {}",
        result.files.len()
    );
}

#[test]
fn load_workspace_discovers_files_from_all_members() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed");

    // workspace has: crates/alpha/src/lib.rs, crates/beta/src/main.rs
    assert_eq!(
        result.files.len(),
        2,
        "expected 2 .rs files across workspace members, got {}",
        result.files.len()
    );

    // Verify files come from different crate directories.
    let paths: Vec<String> = result
        .files
        .iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();

    let has_alpha = paths.iter().any(|p| p.contains("alpha"));
    let has_beta = paths.iter().any(|p| p.contains("beta"));
    assert!(has_alpha, "should discover files from alpha crate: {:?}", paths);
    assert!(has_beta, "should discover files from beta crate: {:?}", paths);
}

#[test]
fn load_project_with_no_source_files_returns_empty_files() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    std::fs::write(
        tmp.path().join("Cargo.toml"),
        r#"[package]
name = "no-src"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let result = load_project(tmp.path()).expect("load should succeed");
    assert!(
        result.files.is_empty(),
        "expected no files for crate without src/, got {}",
        result.files.len()
    );
}

#[test]
fn load_project_file_hash_is_deterministic() {
    let path = fixture_path("single_crate");
    let result1 = load_project(&path).expect("first load should succeed");
    let result2 = load_project(&path).expect("second load should succeed");

    assert_eq!(result1.files.len(), result2.files.len());

    // Build a map of path -> hash for easy comparison.
    let hashes1: std::collections::HashMap<_, _> = result1
        .files
        .iter()
        .map(|f| (f.path.clone(), f.hash.clone()))
        .collect();
    let hashes2: std::collections::HashMap<_, _> = result2
        .files
        .iter()
        .map(|f| (f.path.clone(), f.hash.clone()))
        .collect();

    for (path, hash1) in &hashes1 {
        let hash2 = hashes2
            .get(path)
            .unwrap_or_else(|| panic!("path {:?} missing from second load", path));
        assert_eq!(
            hash1, hash2,
            "hash should be deterministic for {:?}",
            path
        );
    }
}

#[test]
fn load_project_file_line_counts_are_accurate() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "line-count-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    std::fs::create_dir_all(root.join("src")).unwrap();
    // Write a file with exactly 7 lines (trailing newline).
    std::fs::write(
        root.join("src/lib.rs"),
        "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7\n",
    )
    .unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.files[0].line_count, 7);
}

// ---------------------------------------------------------------------------
// Module discovery integration tests
// ---------------------------------------------------------------------------

/// Helper: find a module by name in a slice of modules.
fn find_module_by_name<'a>(modules: &'a [ModuleInfo], name: &str) -> Option<&'a ModuleInfo> {
    modules.iter().find(|m| m.name == name)
}

#[test]
fn load_project_discovers_files_in_nested_directories() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "nested-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    let sub = src.join("sub");
    let deep = sub.join("deep");
    std::fs::create_dir_all(&deep).unwrap();

    std::fs::write(src.join("lib.rs"), "mod sub;\n").unwrap();
    std::fs::write(sub.join("mod.rs"), "mod deep;\n").unwrap();
    std::fs::write(deep.join("mod.rs"), "pub fn deep_fn() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(
        result.files.len(),
        3,
        "expected 3 files (lib.rs + sub/mod.rs + sub/deep/mod.rs), got {}",
        result.files.len()
    );
}

#[test]
fn load_project_ignores_non_rs_files_in_src() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "mixed-files"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
    std::fs::write(src.join("data.json"), "{}").unwrap();
    std::fs::write(src.join("notes.md"), "# Notes").unwrap();
    std::fs::write(src.join("config.toml"), "[config]").unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(
        result.files.len(),
        1,
        "only .rs files should be discovered, got {}",
        result.files.len()
    );
}

// ---------------------------------------------------------------------------
// File discovery: exclusion of build.rs, tests/, examples/, benches/
// ---------------------------------------------------------------------------

#[test]
fn load_project_excludes_build_rs_inside_src() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "build-rs-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("lib.rs"), "pub fn lib_fn() {}\n").unwrap();
    // build.rs inside src/ should be excluded from file discovery.
    std::fs::write(src.join("build.rs"), "fn main() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(
        result.files.len(),
        1,
        "build.rs should be excluded, expected 1 file, got {}",
        result.files.len()
    );

    // Verify it is lib.rs, not build.rs.
    assert!(
        result.files[0]
            .path
            .to_string_lossy()
            .ends_with("lib.rs"),
        "the only file should be lib.rs, got {:?}",
        result.files[0].path
    );
}

#[test]
fn load_project_excludes_tests_dir_inside_src() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "tests-dir-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    let tests_dir = src.join("tests");
    std::fs::create_dir_all(&tests_dir).unwrap();
    std::fs::write(src.join("lib.rs"), "pub fn lib_fn() {}\n").unwrap();
    std::fs::write(tests_dir.join("test_helper.rs"), "pub fn helper() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(
        result.files.len(),
        1,
        "tests/ directory should be excluded, expected 1 file, got {}",
        result.files.len()
    );
}

#[test]
fn load_project_excludes_examples_dir_inside_src() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "examples-dir-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    let examples_dir = src.join("examples");
    std::fs::create_dir_all(&examples_dir).unwrap();
    std::fs::write(src.join("lib.rs"), "pub fn lib_fn() {}\n").unwrap();
    std::fs::write(examples_dir.join("demo.rs"), "fn main() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(
        result.files.len(),
        1,
        "examples/ directory should be excluded, expected 1 file, got {}",
        result.files.len()
    );
}

#[test]
fn load_project_excludes_benches_dir_inside_src() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "benches-dir-test"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    let benches_dir = src.join("benches");
    std::fs::create_dir_all(&benches_dir).unwrap();
    std::fs::write(src.join("lib.rs"), "pub fn lib_fn() {}\n").unwrap();
    std::fs::write(benches_dir.join("bench_main.rs"), "fn main() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");
    assert_eq!(
        result.files.len(),
        1,
        "benches/ directory should be excluded, expected 1 file, got {}",
        result.files.len()
    );
}

#[test]
fn load_project_excludes_all_special_dirs_and_build_rs() {
    // Comprehensive test: all excluded items present alongside real source.
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "comprehensive-exclusion"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(src.join("tests")).unwrap();
    std::fs::create_dir_all(src.join("examples")).unwrap();
    std::fs::create_dir_all(src.join("benches")).unwrap();
    std::fs::create_dir_all(src.join("real_module")).unwrap();

    // Real source files
    std::fs::write(src.join("lib.rs"), "mod real_module;\n").unwrap();
    std::fs::write(src.join("real_module/mod.rs"), "pub fn real() {}\n").unwrap();
    std::fs::write(src.join("utils.rs"), "pub fn util() {}\n").unwrap();

    // Files that should be excluded
    std::fs::write(src.join("build.rs"), "fn main() {}\n").unwrap();
    std::fs::write(src.join("tests/unit.rs"), "fn test() {}\n").unwrap();
    std::fs::write(src.join("examples/demo.rs"), "fn main() {}\n").unwrap();
    std::fs::write(src.join("benches/perf.rs"), "fn main() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");

    // Only lib.rs, real_module/mod.rs, utils.rs should be discovered.
    assert_eq!(
        result.files.len(),
        3,
        "expected 3 real source files (excluding build.rs, tests/, examples/, benches/), got {}",
        result.files.len()
    );

    // Verify none of the excluded files are present.
    for f in &result.files {
        let path_str = f.path.to_string_lossy();
        assert!(
            !path_str.contains("build.rs"),
            "build.rs should not appear in files: {:?}",
            f.path
        );
        assert!(
            !path_str.contains("tests"),
            "tests/ files should not appear: {:?}",
            f.path
        );
        assert!(
            !path_str.contains("examples"),
            "examples/ files should not appear: {:?}",
            f.path
        );
        assert!(
            !path_str.contains("benches"),
            "benches/ files should not appear: {:?}",
            f.path
        );
    }
}

// ---------------------------------------------------------------------------
// Comprehensive single crate integration test with all field verification
// ---------------------------------------------------------------------------

#[test]
fn load_project_single_crate_all_fields_verified() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "full-verify"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("lib.rs"),
        "mod helpers;\n\npub fn root_fn() {}\n",
    )
    .unwrap();
    std::fs::write(src.join("helpers.rs"), "pub fn help() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");

    // -- ProjectInfo --
    assert_eq!(result.project.name, "full-verify");
    assert!(!result.project.is_workspace);
    assert_eq!(result.project.crate_ids.len(), 1);

    // -- CrateInfo --
    assert_eq!(result.crates.len(), 1);
    let krate = &result.crates[0];
    assert_eq!(krate.name, "full-verify");
    assert_eq!(krate.crate_type, CrateType::Library);
    assert_eq!(krate.path, root);
    assert_eq!(result.project.crate_ids[0], krate.id);

    // -- ModuleInfo --
    // root + helpers = 2 modules
    assert_eq!(result.modules.len(), 2);

    let root_mod = result
        .modules
        .iter()
        .find(|m| m.parent.is_none())
        .expect("should have root module");
    assert_eq!(root_mod.name, "full_verify");
    assert_eq!(root_mod.path.as_str(), "full_verify");
    assert!(root_mod.file_path.is_some());
    assert_eq!(root_mod.children.len(), 1);

    let helpers_mod = result
        .modules
        .iter()
        .find(|m| m.name == "helpers")
        .expect("should have helpers module");
    assert_eq!(helpers_mod.path.as_str(), "full_verify::helpers");
    assert_eq!(helpers_mod.parent, Some(root_mod.id));
    assert!(helpers_mod.file_path.is_some());
    assert!(helpers_mod.children.is_empty());

    // Crate should reference both modules.
    assert_eq!(krate.module_ids.len(), 2);
    assert!(krate.module_ids.contains(&root_mod.id));
    assert!(krate.module_ids.contains(&helpers_mod.id));

    // -- FileInfo --
    assert_eq!(result.files.len(), 2);

    for f in &result.files {
        // SHA-256 hashes should be valid 64-char hex strings.
        assert_eq!(f.hash.len(), 64, "hash length wrong for {:?}", f.path);
        assert!(
            f.hash.chars().all(|c| c.is_ascii_hexdigit()),
            "hash should be hex for {:?}",
            f.path
        );
        // Line counts should be positive.
        assert!(f.line_count > 0, "line_count should be > 0 for {:?}", f.path);
        // Paths should be .rs files.
        assert_eq!(
            f.path.extension().and_then(|e| e.to_str()),
            Some("rs"),
            "expected .rs extension for {:?}",
            f.path
        );
    }

    // File IDs should be unique.
    let file_ids: std::collections::HashSet<_> = result.files.iter().map(|f| f.id).collect();
    assert_eq!(file_ids.len(), 2, "file IDs should be unique");
}

// ---------------------------------------------------------------------------
// Comprehensive workspace integration test with all field verification
// ---------------------------------------------------------------------------

#[test]
fn load_project_workspace_all_fields_verified() {
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["crate-a", "crate-b"]
resolver = "2"
"#,
    )
    .unwrap();

    // crate-a: library with one module
    let crate_a = root.join("crate-a");
    std::fs::create_dir_all(crate_a.join("src")).unwrap();
    std::fs::write(
        crate_a.join("Cargo.toml"),
        r#"[package]
name = "crate-a"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::write(
        crate_a.join("src/lib.rs"),
        "mod config;\n\npub fn a_fn() {}\n",
    )
    .unwrap();
    std::fs::write(
        crate_a.join("src/config.rs"),
        "pub const PORT: u16 = 8080;\n",
    )
    .unwrap();

    // crate-b: binary
    let crate_b = root.join("crate-b");
    std::fs::create_dir_all(crate_b.join("src")).unwrap();
    std::fs::write(
        crate_b.join("Cargo.toml"),
        r#"[package]
name = "crate-b"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::write(
        crate_b.join("src/main.rs"),
        "fn main() {\n    println!(\"hello\");\n}\n",
    )
    .unwrap();

    let result = load_project(root).expect("load should succeed");

    // -- ProjectInfo --
    assert!(result.project.is_workspace);
    assert!(result.project.crate_ids.len() >= 2);

    // -- CrateInfo --
    assert!(result.crates.len() >= 2);
    let crate_a_info = result
        .crates
        .iter()
        .find(|c| c.name == "crate-a")
        .expect("should have crate-a");
    assert_eq!(crate_a_info.crate_type, CrateType::Library);

    let crate_b_info = result
        .crates
        .iter()
        .find(|c| c.name == "crate-b")
        .expect("should have crate-b");
    assert_eq!(crate_b_info.crate_type, CrateType::Binary);

    // -- ModuleInfo --
    // crate-a: root + config = 2 modules
    // crate-b: root = 1 module
    // Total = 3
    assert_eq!(
        result.modules.len(),
        3,
        "expected 3 modules across workspace, got {}",
        result.modules.len()
    );

    // -- FileInfo --
    // crate-a: lib.rs + config.rs = 2 files
    // crate-b: main.rs = 1 file
    // Total = 3
    assert_eq!(
        result.files.len(),
        3,
        "expected 3 files across workspace, got {}",
        result.files.len()
    );

    // Files should have valid hashes and line counts.
    for f in &result.files {
        assert_eq!(f.hash.len(), 64);
        assert!(f.line_count > 0);
    }

    // Files from both crates should be present.
    let file_paths: Vec<String> = result
        .files
        .iter()
        .map(|f| f.path.to_string_lossy().to_string())
        .collect();
    let has_crate_a_files = file_paths.iter().any(|p| p.contains("crate-a"));
    let has_crate_b_files = file_paths.iter().any(|p| p.contains("crate-b"));
    assert!(has_crate_a_files, "should have files from crate-a: {:?}", file_paths);
    assert!(has_crate_b_files, "should have files from crate-b: {:?}", file_paths);

    // All file IDs should be unique.
    let file_ids: std::collections::HashSet<_> = result.files.iter().map(|f| f.id).collect();
    assert_eq!(file_ids.len(), result.files.len());
}

// ---------------------------------------------------------------------------
// Graceful error handling integration tests
// ---------------------------------------------------------------------------

#[test]
fn load_project_graceful_with_empty_workspace_member() {
    // A workspace where one member has no source files at all.
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[workspace]
members = ["has-src", "no-src"]
resolver = "2"
"#,
    )
    .unwrap();

    // has-src: normal library
    let has_src = root.join("has-src");
    std::fs::create_dir_all(has_src.join("src")).unwrap();
    std::fs::write(
        has_src.join("Cargo.toml"),
        r#"[package]
name = "has-src"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();
    std::fs::write(has_src.join("src/lib.rs"), "pub fn present() {}\n").unwrap();

    // no-src: crate with Cargo.toml but no src/ directory
    let no_src = root.join("no-src");
    std::fs::create_dir_all(&no_src).unwrap();
    std::fs::write(
        no_src.join("Cargo.toml"),
        r#"[package]
name = "no-src"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let result = load_project(root).expect("should succeed despite empty crate");

    assert!(result.project.is_workspace);

    // has-src should contribute files, no-src should not.
    assert!(
        result.files.len() >= 1,
        "at least 1 file from has-src, got {}",
        result.files.len()
    );

    // The no-src crate should still be discovered (registered with fallback).
    let names: Vec<&str> = result.crates.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"has-src"), "expected has-src crate");
    assert!(names.contains(&"no-src"), "expected no-src crate");
}

// ---------------------------------------------------------------------------
// Module deduplication for dual-target crates
// ---------------------------------------------------------------------------

#[test]
fn dual_target_has_separate_root_modules() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    // With deduplication, we expect:
    // - lib root module (from lib.rs) -- root, no parent
    // - bin root module (from main.rs) -- root, no parent
    // - shared module (from shared.rs) -- deduplicated, single entry
    // Total: 3 unique ModuleInfo entries
    let lib_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Library)
        .expect("should have a library crate target");
    let bin_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Binary)
        .expect("should have a binary crate target");

    // Both crates should exist and be distinct.
    assert_ne!(lib_crate.id, bin_crate.id);

    // Find root modules -- there should be two distinct roots.
    let root_modules: Vec<&ModuleInfo> = result
        .modules
        .iter()
        .filter(|m| m.parent.is_none())
        .collect();
    assert_eq!(
        root_modules.len(),
        2,
        "expected 2 root modules (lib + bin), got {}",
        root_modules.len()
    );

    // One root should point to lib.rs, the other to main.rs.
    let lib_root = root_modules
        .iter()
        .find(|m| {
            m.file_path
                .as_ref()
                .map_or(false, |p| p.to_string_lossy().ends_with("lib.rs"))
        })
        .expect("should have a lib.rs root module");
    let bin_root = root_modules
        .iter()
        .find(|m| {
            m.file_path
                .as_ref()
                .map_or(false, |p| p.to_string_lossy().ends_with("main.rs"))
        })
        .expect("should have a main.rs root module");

    assert_ne!(lib_root.id, bin_root.id, "root modules should have distinct IDs");
}

#[test]
fn dual_target_shared_module_is_deduplicated() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    // The "shared" module should appear exactly once in result.modules,
    // not twice (once per target).
    let shared_modules: Vec<&ModuleInfo> = result
        .modules
        .iter()
        .filter(|m| m.name == "shared")
        .collect();

    assert_eq!(
        shared_modules.len(),
        1,
        "expected exactly 1 'shared' module entry (deduplicated), got {}",
        shared_modules.len()
    );

    let shared = shared_modules[0];
    assert!(
        shared.file_path.is_some(),
        "shared module should have a file_path"
    );
    assert!(
        shared
            .file_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .ends_with("shared.rs"),
        "shared module should point to shared.rs"
    );
}

#[test]
fn dual_target_both_crates_reference_shared_module() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    let lib_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Library)
        .expect("should have a library crate target");
    let bin_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Binary)
        .expect("should have a binary crate target");

    let shared = result
        .modules
        .iter()
        .find(|m| m.name == "shared")
        .expect("should have a shared module");

    // Both crate targets should reference the shared module in their module_ids.
    assert!(
        lib_crate.module_ids.contains(&shared.id),
        "library crate should reference the shared module (id {:?}), but module_ids = {:?}",
        shared.id,
        lib_crate.module_ids
    );
    assert!(
        bin_crate.module_ids.contains(&shared.id),
        "binary crate should reference the shared module (id {:?}), but module_ids = {:?}",
        shared.id,
        bin_crate.module_ids
    );
}

#[test]
fn dual_target_total_module_count() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    // Expected unique modules:
    // 1. lib root (lib.rs) -- parent: None
    // 2. bin root (main.rs) -- parent: None
    // 3. shared (shared.rs) -- deduplicated, single entry
    assert_eq!(
        result.modules.len(),
        3,
        "expected 3 unique module entries (lib root + bin root + shared), got {}",
        result.modules.len()
    );
}

#[test]
fn dual_target_lib_root_children_reference_shared() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    let shared = result
        .modules
        .iter()
        .find(|m| m.name == "shared")
        .expect("should have shared module");

    // The library root's children should include the shared module.
    let lib_root = result
        .modules
        .iter()
        .find(|m| {
            m.parent.is_none()
                && m.file_path
                    .as_ref()
                    .map_or(false, |p| p.to_string_lossy().ends_with("lib.rs"))
        })
        .expect("should have lib root");

    assert!(
        lib_root.children.contains(&shared.id),
        "lib root children should contain shared module ID {:?}, got {:?}",
        shared.id,
        lib_root.children
    );
}

#[test]
fn dual_target_bin_root_children_reference_shared() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    let shared = result
        .modules
        .iter()
        .find(|m| m.name == "shared")
        .expect("should have shared module");

    // The binary root's children should also reference the same shared module ID
    // (the deduplicated one), not a dangling ID.
    let bin_root = result
        .modules
        .iter()
        .find(|m| {
            m.parent.is_none()
                && m.file_path
                    .as_ref()
                    .map_or(false, |p| p.to_string_lossy().ends_with("main.rs"))
        })
        .expect("should have bin root");

    assert!(
        bin_root.children.contains(&shared.id),
        "bin root children should reference the deduplicated shared module ID {:?}, got {:?}",
        shared.id,
        bin_root.children
    );
}

#[test]
fn dual_target_all_module_ids_exist_in_modules() {
    let path = fixture_path("single_crate_both");
    let result = load_project(&path).expect("load_project should succeed");

    let all_module_ids: std::collections::HashSet<_> =
        result.modules.iter().map(|m| m.id).collect();

    // Every module ID referenced by any crate must exist in result.modules.
    for krate in &result.crates {
        for mid in &krate.module_ids {
            assert!(
                all_module_ids.contains(mid),
                "crate {:?} (type {:?}) references module ID {:?} which does not exist in result.modules",
                krate.name,
                krate.crate_type,
                mid
            );
        }
    }

    // Every child ID in every module must also exist in result.modules.
    for m in &result.modules {
        for child_id in &m.children {
            assert!(
                all_module_ids.contains(child_id),
                "module {:?} has child ID {:?} which does not exist in result.modules",
                m.name,
                child_id
            );
        }
    }
}

#[test]
fn dual_target_module_dedup_with_deep_shared_tree() {
    // Create a dual-target project where both lib.rs and main.rs declare
    // `mod parent;` and parent.rs declares `mod child;`. The entire subtree
    // (parent + child) should be deduplicated.
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "deep-dual"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(src.join("parent")).unwrap();

    std::fs::write(src.join("lib.rs"), "mod parent;\npub fn lib_fn() {}\n").unwrap();
    std::fs::write(src.join("main.rs"), "mod parent;\nfn main() {}\n").unwrap();
    std::fs::write(src.join("parent.rs"), "mod child;\npub fn parent_fn() {}\n").unwrap();
    std::fs::write(
        src.join("parent").join("child.rs"),
        "pub fn child_fn() {}\n",
    )
    .unwrap();

    let result = load_project(root).expect("load should succeed");

    // Unique modules: lib_root, bin_root, parent, child = 4
    assert_eq!(
        result.modules.len(),
        4,
        "expected 4 unique modules (lib_root + bin_root + parent + child), got {}",
        result.modules.len()
    );

    // "parent" and "child" should each appear exactly once.
    let parent_count = result.modules.iter().filter(|m| m.name == "parent").count();
    let child_count = result.modules.iter().filter(|m| m.name == "child").count();
    assert_eq!(parent_count, 1, "parent module should be deduplicated to 1 entry");
    assert_eq!(child_count, 1, "child module should be deduplicated to 1 entry");

    // Both crate targets should reference parent.
    let parent_mod = result.modules.iter().find(|m| m.name == "parent").unwrap();
    let lib_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Library)
        .unwrap();
    let bin_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Binary)
        .unwrap();

    assert!(
        lib_crate.module_ids.contains(&parent_mod.id),
        "lib crate should reference parent module"
    );
    assert!(
        bin_crate.module_ids.contains(&parent_mod.id),
        "bin crate should reference parent module"
    );

    // Verify referential integrity: all children and module_ids point to real modules.
    let all_ids: std::collections::HashSet<_> = result.modules.iter().map(|m| m.id).collect();
    for m in &result.modules {
        for child_id in &m.children {
            assert!(
                all_ids.contains(child_id),
                "module {:?} has dangling child ID {:?}",
                m.name,
                child_id
            );
        }
    }
    for krate in &result.crates {
        for mid in &krate.module_ids {
            assert!(
                all_ids.contains(mid),
                "crate {:?} has dangling module ID {:?}",
                krate.name,
                mid
            );
        }
    }
}

#[test]
fn single_target_crate_has_no_dedup_side_effects() {
    // A single-target crate (library only) should work exactly as before.
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    // single_crate has lib.rs -> utils, models = 3 modules.
    assert_eq!(
        result.modules.len(),
        3,
        "single-target crate should not be affected by dedup logic"
    );

    let krate = &result.crates[0];
    assert_eq!(
        krate.module_ids.len(),
        3,
        "all 3 modules should be in the crate's module_ids"
    );
}

#[test]
fn dual_target_no_shared_modules() {
    // When lib.rs and main.rs have no overlapping mod declarations,
    // dedup should not remove anything.
    let tmp = tempfile::tempdir().expect("failed to create temp dir");
    let root = tmp.path();

    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "no-overlap"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();

    std::fs::write(src.join("lib.rs"), "mod lib_only;\npub fn lib_fn() {}\n").unwrap();
    std::fs::write(src.join("main.rs"), "mod bin_only;\nfn main() {}\n").unwrap();
    std::fs::write(src.join("lib_only.rs"), "pub fn lo() {}\n").unwrap();
    std::fs::write(src.join("bin_only.rs"), "pub fn bo() {}\n").unwrap();

    let result = load_project(root).expect("load should succeed");

    // lib_root + lib_only + bin_root + bin_only = 4 unique modules (no overlap).
    assert_eq!(
        result.modules.len(),
        4,
        "no deduplication should occur when modules do not overlap"
    );

    // Verify each crate has 2 module_ids (root + its own child).
    let lib_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Library)
        .unwrap();
    let bin_crate = result
        .crates
        .iter()
        .find(|c| c.crate_type == CrateType::Binary)
        .unwrap();

    assert_eq!(lib_crate.module_ids.len(), 2, "lib should have root + lib_only");
    assert_eq!(bin_crate.module_ids.len(), 2, "bin should have root + bin_only");
}

#[test]
fn single_crate_modules_has_correct_count() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    // single_crate/src/lib.rs declares `mod utils;` and `mod models;`.
    // Expected modules: root (my_single_crate) + utils + models = 3.
    assert_eq!(
        result.modules.len(),
        3,
        "expected 3 modules (root + utils + models), got {}",
        result.modules.len()
    );
}

#[test]
fn single_crate_modules_has_root_module() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    // The root module name uses underscore normalization of the crate name.
    let root = find_module_by_name(&result.modules, "my_single_crate")
        .expect("should have root module named my_single_crate");

    assert_eq!(root.path.as_str(), "my_single_crate");
    assert!(root.parent.is_none(), "root module should have no parent");
    assert!(root.file_path.is_some(), "root module should have a file_path");
    assert!(
        root.file_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .ends_with("lib.rs"),
        "root module file should be lib.rs, got {:?}",
        root.file_path
    );
}

#[test]
fn single_crate_modules_has_child_modules() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    let root = find_module_by_name(&result.modules, "my_single_crate")
        .expect("should have root module");

    // Root should have two children.
    assert_eq!(
        root.children.len(),
        2,
        "root module should have 2 children, got {}",
        root.children.len()
    );

    // Verify the utils child module.
    let utils = find_module_by_name(&result.modules, "utils")
        .expect("should have 'utils' module");
    assert_eq!(utils.path.as_str(), "my_single_crate::utils");
    assert_eq!(
        utils.parent,
        Some(root.id),
        "utils parent should be the root module"
    );
    assert!(
        utils.file_path.is_some(),
        "utils module should have a file_path"
    );
    assert!(
        utils
            .file_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .ends_with("utils.rs"),
        "utils file should be utils.rs, got {:?}",
        utils.file_path
    );

    // Verify the models child module.
    let models = find_module_by_name(&result.modules, "models")
        .expect("should have 'models' module");
    assert_eq!(models.path.as_str(), "my_single_crate::models");
    assert_eq!(
        models.parent,
        Some(root.id),
        "models parent should be the root module"
    );
    assert!(
        models.file_path.is_some(),
        "models module should have a file_path"
    );
}

#[test]
fn single_crate_module_ids_match_crate_module_ids() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    let krate = &result.crates[0];

    // The crate's module_ids should contain exactly the IDs of the discovered modules.
    assert_eq!(
        krate.module_ids.len(),
        result.modules.len(),
        "crate module_ids count should match modules count"
    );

    for m in &result.modules {
        assert!(
            krate.module_ids.contains(&m.id),
            "crate module_ids should contain module {:?} (id {:?})",
            m.name,
            m.id
        );
    }
}

#[test]
fn single_crate_module_ids_are_unique() {
    let path = fixture_path("single_crate");
    let result = load_project(&path).expect("load_project should succeed");

    let ids: std::collections::HashSet<_> = result.modules.iter().map(|m| m.id).collect();
    assert_eq!(
        ids.len(),
        result.modules.len(),
        "all module IDs should be unique"
    );
}

// ---------------------------------------------------------------------------
// Module discovery: workspace fixture
// ---------------------------------------------------------------------------

#[test]
fn workspace_modules_has_correct_count() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed");

    // alpha/src/lib.rs has no mod declarations -> 1 root module (alpha).
    // beta/src/main.rs has no mod declarations -> 1 root module (beta).
    // Total: 2 modules.
    assert_eq!(
        result.modules.len(),
        2,
        "expected 2 modules (alpha root + beta root), got {}",
        result.modules.len()
    );
}

#[test]
fn workspace_modules_has_alpha_root() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed");

    let alpha_root = find_module_by_name(&result.modules, "alpha")
        .expect("should have root module for crate 'alpha'");

    assert_eq!(alpha_root.path.as_str(), "alpha");
    assert!(alpha_root.parent.is_none(), "alpha root should have no parent");
    assert!(
        alpha_root.children.is_empty(),
        "alpha root should have no children (no mod declarations in lib.rs)"
    );
    assert!(alpha_root.file_path.is_some());
    assert!(
        alpha_root
            .file_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .ends_with("lib.rs"),
        "alpha root file should be lib.rs, got {:?}",
        alpha_root.file_path
    );
}

#[test]
fn workspace_modules_has_beta_root() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed");

    let beta_root = find_module_by_name(&result.modules, "beta")
        .expect("should have root module for crate 'beta'");

    assert_eq!(beta_root.path.as_str(), "beta");
    assert!(beta_root.parent.is_none(), "beta root should have no parent");
    assert!(
        beta_root.children.is_empty(),
        "beta root should have no children (no mod declarations in main.rs)"
    );
    assert!(beta_root.file_path.is_some());
    assert!(
        beta_root
            .file_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .ends_with("main.rs"),
        "beta root file should be main.rs, got {:?}",
        beta_root.file_path
    );
}

#[test]
fn workspace_crate_module_ids_are_populated() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed");

    let alpha_crate = result.crates.iter().find(|c| c.name == "alpha").unwrap();
    assert_eq!(
        alpha_crate.module_ids.len(),
        1,
        "alpha crate should have 1 module_id (its root module)"
    );

    let beta_crate = result.crates.iter().find(|c| c.name == "beta").unwrap();
    assert_eq!(
        beta_crate.module_ids.len(),
        1,
        "beta crate should have 1 module_id (its root module)"
    );

    // Each crate's module_ids should reference modules in the result.
    let all_module_ids: std::collections::HashSet<_> =
        result.modules.iter().map(|m| m.id).collect();

    for mid in &alpha_crate.module_ids {
        assert!(
            all_module_ids.contains(mid),
            "alpha module_id {:?} should exist in result.modules",
            mid
        );
    }
    for mid in &beta_crate.module_ids {
        assert!(
            all_module_ids.contains(mid),
            "beta module_id {:?} should exist in result.modules",
            mid
        );
    }
}

#[test]
fn workspace_module_ids_are_unique() {
    let path = fixture_path("workspace");
    let result = load_project(&path).expect("load_project should succeed");

    let ids: std::collections::HashSet<_> = result.modules.iter().map(|m| m.id).collect();
    assert_eq!(
        ids.len(),
        result.modules.len(),
        "all module IDs across workspace should be unique"
    );
}

// ---------------------------------------------------------------------------
// Module discovery: missing_module fixture
// ---------------------------------------------------------------------------

#[test]
fn missing_module_modules_has_correct_count() {
    let path = fixture_path("malformed/missing_module");
    let result = load_project(&path).expect("load_project should succeed");

    // lib.rs declares `mod nonexistent;` but no file exists.
    // Expected: root (missing_module_crate) + nonexistent = 2 modules.
    assert_eq!(
        result.modules.len(),
        2,
        "expected 2 modules (root + nonexistent), got {}",
        result.modules.len()
    );
}

#[test]
fn missing_module_has_root_module() {
    let path = fixture_path("malformed/missing_module");
    let result = load_project(&path).expect("load_project should succeed");

    let root = find_module_by_name(&result.modules, "missing_module_crate")
        .expect("should have root module named missing_module_crate");

    assert_eq!(root.path.as_str(), "missing_module_crate");
    assert!(root.parent.is_none());
    assert_eq!(
        root.children.len(),
        1,
        "root should have 1 child (the nonexistent module)"
    );
    assert!(root.file_path.is_some());
}

#[test]
fn missing_module_records_module_with_no_file_path() {
    let path = fixture_path("malformed/missing_module");
    let result = load_project(&path).expect("load_project should succeed");

    let root = find_module_by_name(&result.modules, "missing_module_crate")
        .expect("should have root module");

    let missing = find_module_by_name(&result.modules, "nonexistent")
        .expect("should have 'nonexistent' module entry despite missing file");

    assert_eq!(missing.path.as_str(), "missing_module_crate::nonexistent");
    assert_eq!(
        missing.parent,
        Some(root.id),
        "nonexistent module parent should be the root"
    );
    assert!(
        missing.file_path.is_none(),
        "nonexistent module should have file_path: None since the file does not exist"
    );
    assert!(
        missing.children.is_empty(),
        "nonexistent module should have no children"
    );
}

#[test]
fn missing_module_crate_module_ids_are_populated() {
    let path = fixture_path("malformed/missing_module");
    let result = load_project(&path).expect("load_project should succeed");

    let krate = &result.crates[0];
    assert_eq!(
        krate.module_ids.len(),
        2,
        "crate should track both the root module and the nonexistent module"
    );

    for m in &result.modules {
        assert!(
            krate.module_ids.contains(&m.id),
            "crate module_ids should contain module {:?}",
            m.name
        );
    }
}
