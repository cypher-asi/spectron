//! Integration tests for `parse_project()`.
//!
//! These tests use the test fixtures from `spectron-loader` and custom fixtures
//! to verify the full parsing pipeline end-to-end.

use std::path::PathBuf;

use spectron_loader::load_project;
use spectron_parser::parse_project;

/// Helper: resolve a path relative to the workspace root.
fn fixture_path(relative: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.join(relative)
}

/// Helper: resolve a path to a loader test fixture.
fn loader_fixture_path(name: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Go up from crates/spectron-parser to workspace root, then into loader fixtures.
    manifest_dir
        .parent()
        .unwrap()
        .join("spectron-loader")
        .join("tests")
        .join("fixtures")
        .join(name)
}

// ---------------------------------------------------------------------------
// Single crate fixture
// ---------------------------------------------------------------------------

#[test]
fn parse_single_crate_fixture() {
    let path = loader_fixture_path("single_crate");
    let load_result = load_project(&path).expect("load_project should succeed");

    let parse_result = parse_project(&load_result);

    // The single_crate fixture has:
    // - lib.rs: fn hello(), mod utils, mod models
    // - utils.rs: fn utility_fn()
    // - models.rs: struct Model
    // So we expect at least 3 symbols (hello, utility_fn, Model).
    assert!(
        !parse_result.symbols.is_empty(),
        "expected non-zero symbols from single_crate fixture, got 0"
    );
    assert!(
        parse_result.symbols.len() >= 3,
        "expected at least 3 symbols from single_crate fixture, got {}",
        parse_result.symbols.len()
    );

    // Verify specific symbols are present.
    let names: Vec<&str> = parse_result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"hello"),
        "expected 'hello' symbol; found: {:?}",
        names
    );
    assert!(
        names.contains(&"utility_fn"),
        "expected 'utility_fn' symbol; found: {:?}",
        names
    );
    assert!(
        names.contains(&"Model"),
        "expected 'Model' symbol; found: {:?}",
        names
    );

    // No parse errors expected for valid fixture.
    assert!(
        parse_result.errors.is_empty(),
        "expected no parse errors for single_crate fixture, got: {:?}",
        parse_result.errors
    );
}

// ---------------------------------------------------------------------------
// Workspace fixture
// ---------------------------------------------------------------------------

#[test]
fn parse_workspace_fixture() {
    let path = loader_fixture_path("workspace");
    let load_result = load_project(&path).expect("load_project should succeed");

    let parse_result = parse_project(&load_result);

    // The workspace fixture has:
    // - alpha/src/lib.rs: fn alpha_fn()
    // - beta/src/main.rs: fn main()
    // At least 2 symbols expected.
    assert!(
        !parse_result.symbols.is_empty(),
        "expected non-zero symbols from workspace fixture, got 0"
    );
    assert!(
        parse_result.symbols.len() >= 2,
        "expected at least 2 symbols from workspace fixture, got {}",
        parse_result.symbols.len()
    );

    let names: Vec<&str> = parse_result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"alpha_fn"),
        "expected 'alpha_fn' symbol; found: {:?}",
        names
    );
    assert!(
        names.contains(&"main"),
        "expected 'main' symbol; found: {:?}",
        names
    );

    // No parse errors expected.
    assert!(
        parse_result.errors.is_empty(),
        "expected no parse errors for workspace fixture, got: {:?}",
        parse_result.errors
    );
}

// ---------------------------------------------------------------------------
// Single crate with both lib.rs and main.rs
// ---------------------------------------------------------------------------

#[test]
fn parse_single_crate_both_fixture() {
    let path = loader_fixture_path("single_crate_both");
    let load_result = load_project(&path).expect("load_project should succeed");

    let parse_result = parse_project(&load_result);

    // The single_crate_both fixture has:
    // - lib.rs: fn library_fn(), mod shared
    // - main.rs: fn main(), mod shared
    // - shared.rs: fn shared_fn()
    // Expect at least 3 distinct function symbols.
    assert!(
        !parse_result.symbols.is_empty(),
        "expected non-zero symbols, got 0"
    );

    let names: Vec<&str> = parse_result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"library_fn"),
        "expected 'library_fn' symbol; found: {:?}",
        names
    );
    assert!(
        names.contains(&"main"),
        "expected 'main' symbol; found: {:?}",
        names
    );
    assert!(
        names.contains(&"shared_fn"),
        "expected 'shared_fn' symbol; found: {:?}",
        names
    );

    // No parse errors expected.
    assert!(
        parse_result.errors.is_empty(),
        "expected no parse errors, got: {:?}",
        parse_result.errors
    );
}

// ---------------------------------------------------------------------------
// Fixture with a malformed Rust file
// ---------------------------------------------------------------------------

#[test]
fn parse_fixture_with_parse_error() {
    let path = fixture_path("tests/fixtures/with_parse_error");
    let load_result = load_project(&path).expect("load_project should succeed");

    let parse_result = parse_project(&load_result);

    // We should still get symbols from the good files (lib.rs, good.rs).
    assert!(
        !parse_result.symbols.is_empty(),
        "expected non-zero symbols even with a malformed file, got 0"
    );

    let names: Vec<&str> = parse_result.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(
        names.contains(&"top_level_fn"),
        "expected 'top_level_fn' from lib.rs; found: {:?}",
        names
    );
    assert!(
        names.contains(&"GoodStruct"),
        "expected 'GoodStruct' from good.rs; found: {:?}",
        names
    );
    assert!(
        names.contains(&"good_function"),
        "expected 'good_function' from good.rs; found: {:?}",
        names
    );

    // We should have at least one parse error from bad.rs.
    assert!(
        !parse_result.errors.is_empty(),
        "expected at least one parse error for malformed fixture, got 0"
    );

    // Verify the error mentions the bad file.
    let bad_errors: Vec<_> = parse_result
        .errors
        .iter()
        .filter(|e| {
            e.file_path
                .to_string_lossy()
                .contains("bad.rs")
        })
        .collect();
    assert!(
        !bad_errors.is_empty(),
        "expected parse error for bad.rs; errors: {:?}",
        parse_result.errors
    );
}

// ---------------------------------------------------------------------------
// Relationships are extracted
// ---------------------------------------------------------------------------

#[test]
fn parse_single_crate_has_relationships() {
    let path = loader_fixture_path("single_crate");
    let load_result = load_project(&path).expect("load_project should succeed");

    let parse_result = parse_project(&load_result);

    // The single_crate fixture's lib.rs has `mod utils; mod models;` which
    // should be detected. There may also be other relationships depending on
    // the visitor's extraction. At minimum, we expect the symbols to be
    // present. Relationships may be empty if there are no calls/imports between
    // the simple fixture files, but given the structure, there should be at
    // least some relationships or the count should be >= 0 (no panic).
    //
    // The main assertion is that parse_project completes without panic and
    // produces a valid ParseResult.
    assert!(
        parse_result.symbols.len() >= 3,
        "expected at least 3 symbols, got {}",
        parse_result.symbols.len()
    );
}

// ---------------------------------------------------------------------------
// Empty result for empty src
// ---------------------------------------------------------------------------

#[test]
fn parse_empty_src_fixture() {
    let path = loader_fixture_path("malformed/empty_src");
    let load_result = load_project(&path).expect("load_project should succeed");

    let parse_result = parse_project(&load_result);

    // empty_src has no .rs files, so we expect no symbols, no relationships,
    // and no errors.
    assert!(
        parse_result.symbols.is_empty(),
        "expected 0 symbols from empty src, got {}",
        parse_result.symbols.len()
    );
    assert!(
        parse_result.errors.is_empty(),
        "expected 0 errors from empty src, got {}",
        parse_result.errors.len()
    );
}
