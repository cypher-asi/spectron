//! File discovery and hashing for crate source directories.
//!
//! Walks each crate's `src/` directory to discover `.rs` files, computing a
//! SHA-256 content hash and line count for each one. Non-UTF8 paths are
//! skipped with a warning. Symlinks are followed (walkdir default).

use std::path::Path;

use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use spectron_core::id::IdGenerator;
use spectron_core::FileInfo;

/// Discover all `.rs` files under a crate's `src/` directory.
///
/// For each discovered file, computes its SHA-256 hash and line count, then
/// returns a [`FileInfo`] with a freshly generated [`FileId`].
///
/// # Edge cases
///
/// - If `crate_path/src/` does not exist, returns an empty `Vec`.
/// - Non-UTF8 file paths are skipped with a tracing warning.
/// - Unreadable files are skipped with a tracing warning.
/// - Symlinks are followed (walkdir default behavior).
pub fn discover_files(id_gen: &IdGenerator, crate_path: &Path) -> Vec<FileInfo> {
    let src_dir = crate_path.join("src");
    if !src_dir.exists() {
        tracing::debug!(
            path = %crate_path.display(),
            "no src/ directory found, skipping file discovery"
        );
        return Vec::new();
    }

    let mut files = Vec::new();

    for entry in WalkDir::new(&src_dir)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| !is_excluded_directory(e))
    {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "error while walking src/ directory"
                );
                continue;
            }
        };

        // Only process regular files (or symlinks resolved to files).
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Skip non-.rs files.
        match path.extension().and_then(|ext| ext.to_str()) {
            Some("rs") => {}
            _ => continue,
        }

        // Skip build.rs files (should not be treated as regular source).
        if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
            if file_name == "build.rs" {
                tracing::debug!(
                    path = %path.display(),
                    "skipping build.rs file"
                );
                continue;
            }
        }

        // Verify the path is valid UTF-8.
        if path.to_str().is_none() {
            tracing::warn!(
                path = ?path,
                "skipping file with non-UTF8 path"
            );
            continue;
        }

        match read_file_info(id_gen, path) {
            Ok(info) => files.push(info),
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "failed to read source file, skipping"
                );
            }
        }
    }

    files
}

/// Check whether a directory entry should be excluded from the source walk.
///
/// Excluded directories (per spec Phase 1):
/// - `tests/`
/// - `examples/`
/// - `benches/`
///
/// These directories are excluded even when they appear inside `src/` to avoid
/// picking up non-module source files that belong to test harnesses, examples,
/// or benchmarks.
fn is_excluded_directory(entry: &walkdir::DirEntry) -> bool {
    if !entry.file_type().is_dir() {
        return false;
    }
    match entry.file_name().to_str() {
        Some("tests") | Some("examples") | Some("benches") => {
            tracing::debug!(
                path = %entry.path().display(),
                "excluding directory from file discovery"
            );
            true
        }
        _ => false,
    }
}

/// Read a single file and produce a [`FileInfo`] with SHA-256 hash and line count.
fn read_file_info(
    id_gen: &IdGenerator,
    path: &Path,
) -> Result<FileInfo, std::io::Error> {
    let content = std::fs::read(path)?;

    let hash = compute_sha256(&content);
    let line_count = count_lines(&content);

    let id = id_gen.next_file();
    Ok(FileInfo::new(id, path, hash, line_count))
}

/// Compute the SHA-256 hex digest of the given bytes.
fn compute_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    hex_encode(&result)
}

/// Encode a byte slice as a lowercase hex string.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Count the number of lines in a byte slice.
///
/// A file with no trailing newline still counts its last line.
/// An empty file has 0 lines.
fn count_lines(data: &[u8]) -> u32 {
    if data.is_empty() {
        return 0;
    }
    // Count newline characters, then add 1 if the file does not end with a newline.
    let newlines = bytecount(data, b'\n');
    if data.last() == Some(&b'\n') {
        newlines
    } else {
        newlines + 1
    }
}

/// Count occurrences of a byte in a slice.
fn bytecount(data: &[u8], needle: u8) -> u32 {
    data.iter().filter(|&&b| b == needle).count() as u32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // Unit tests for compute_sha256
    // -----------------------------------------------------------------------

    #[test]
    fn sha256_empty_input() {
        let hash = compute_sha256(b"");
        // Well-known SHA-256 of empty input.
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn sha256_known_value() {
        let hash = compute_sha256(b"hello\n");
        // SHA-256 of "hello\n".
        assert_eq!(
            hash,
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
        );
    }

    #[test]
    fn sha256_deterministic() {
        let data = b"pub fn foo() {}\n";
        let h1 = compute_sha256(data);
        let h2 = compute_sha256(data);
        assert_eq!(h1, h2);
    }

    #[test]
    fn sha256_different_content_different_hash() {
        let h1 = compute_sha256(b"aaa");
        let h2 = compute_sha256(b"bbb");
        assert_ne!(h1, h2);
    }

    // -----------------------------------------------------------------------
    // Unit tests for count_lines
    // -----------------------------------------------------------------------

    #[test]
    fn count_lines_empty() {
        assert_eq!(count_lines(b""), 0);
    }

    #[test]
    fn count_lines_single_line_no_newline() {
        assert_eq!(count_lines(b"hello"), 1);
    }

    #[test]
    fn count_lines_single_line_with_newline() {
        assert_eq!(count_lines(b"hello\n"), 1);
    }

    #[test]
    fn count_lines_multiple_lines() {
        assert_eq!(count_lines(b"line1\nline2\nline3\n"), 3);
    }

    #[test]
    fn count_lines_multiple_lines_no_trailing_newline() {
        assert_eq!(count_lines(b"line1\nline2\nline3"), 3);
    }

    #[test]
    fn count_lines_blank_lines() {
        assert_eq!(count_lines(b"\n\n\n"), 3);
    }

    // -----------------------------------------------------------------------
    // Unit tests for hex_encode
    // -----------------------------------------------------------------------

    #[test]
    fn hex_encode_empty() {
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn hex_encode_bytes() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0xab, 0x10]), "00ffab10");
    }

    // -----------------------------------------------------------------------
    // Integration-style tests for discover_files
    // -----------------------------------------------------------------------

    #[test]
    fn discover_files_returns_empty_for_missing_src_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert!(files.is_empty());
    }

    #[test]
    fn discover_files_returns_empty_for_no_rs_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("readme.txt"), "not rust").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert!(files.is_empty());
    }

    #[test]
    fn discover_files_finds_rs_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(src.join("utils.rs"), "pub fn util() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 2, "expected 2 .rs files, got {}", files.len());

        // Each file should have a non-empty hash and correct line count.
        for f in &files {
            assert!(!f.hash.is_empty(), "hash should not be empty");
            assert_eq!(f.hash.len(), 64, "SHA-256 hex should be 64 chars");
            assert!(f.line_count > 0, "line count should be > 0");
            assert!(
                f.path.extension().and_then(|e| e.to_str()) == Some("rs"),
                "file should have .rs extension"
            );
        }
    }

    #[test]
    fn discover_files_recurses_into_subdirectories() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let sub = src.join("submod");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(src.join("lib.rs"), "mod submod;\n").unwrap();
        std::fs::write(sub.join("mod.rs"), "pub fn sub() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 2, "expected 2 files (lib.rs + submod/mod.rs)");
    }

    #[test]
    fn discover_files_skips_non_rs_files() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(src.join("notes.txt"), "not code").unwrap();
        std::fs::write(src.join("data.json"), "{}").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 1, "only .rs files should be discovered");
    }

    #[test]
    fn discover_files_computes_correct_hash() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        let content = b"pub fn foo() {}\n";
        std::fs::write(src.join("lib.rs"), content).unwrap();

        let expected_hash = compute_sha256(content);

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].hash, expected_hash);
    }

    #[test]
    fn discover_files_computes_correct_line_count() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();

        // 5 lines with trailing newline
        let content = "line 1\nline 2\nline 3\nline 4\nline 5\n";
        std::fs::write(src.join("lib.rs"), content).unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].line_count, 5);
    }

    #[test]
    fn discover_files_assigns_unique_file_ids() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "// lib\n").unwrap();
        std::fs::write(src.join("a.rs"), "// a\n").unwrap();
        std::fs::write(src.join("b.rs"), "// b\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 3);

        let ids: std::collections::HashSet<_> = files.iter().map(|f| f.id).collect();
        assert_eq!(ids.len(), 3, "all file IDs should be unique");
    }

    #[test]
    fn discover_files_handles_empty_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].line_count, 0);
        // Should be SHA-256 of empty string
        assert_eq!(
            files[0].hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    // -----------------------------------------------------------------------
    // Exclusion tests: build.rs, tests/, examples/, benches/
    // -----------------------------------------------------------------------

    #[test]
    fn discover_files_excludes_build_rs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(src.join("build.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(
            files.len(),
            1,
            "build.rs should be excluded, got {} files",
            files.len()
        );
        assert!(
            files[0].path.to_string_lossy().ends_with("lib.rs"),
            "only lib.rs should be discovered"
        );
    }

    #[test]
    fn discover_files_excludes_tests_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let tests = src.join("tests");
        std::fs::create_dir_all(&tests).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(tests.join("unit.rs"), "fn test() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(
            files.len(),
            1,
            "tests/ directory should be excluded, got {} files",
            files.len()
        );
    }

    #[test]
    fn discover_files_excludes_examples_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let examples = src.join("examples");
        std::fs::create_dir_all(&examples).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(examples.join("demo.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(
            files.len(),
            1,
            "examples/ directory should be excluded, got {} files",
            files.len()
        );
    }

    #[test]
    fn discover_files_excludes_benches_directory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let benches = src.join("benches");
        std::fs::create_dir_all(&benches).unwrap();
        std::fs::write(src.join("lib.rs"), "pub fn lib() {}\n").unwrap();
        std::fs::write(benches.join("perf.rs"), "fn main() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(
            files.len(),
            1,
            "benches/ directory should be excluded, got {} files",
            files.len()
        );
    }

    #[test]
    fn discover_files_keeps_non_excluded_subdirectories() {
        // Directories that are NOT named tests/examples/benches should still be walked.
        let tmp = tempfile::tempdir().expect("tempdir");
        let src = tmp.path().join("src");
        let sub = src.join("helpers");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(src.join("lib.rs"), "mod helpers;\n").unwrap();
        std::fs::write(sub.join("mod.rs"), "pub fn help() {}\n").unwrap();

        let id_gen = IdGenerator::new();
        let files = discover_files(&id_gen, tmp.path());
        assert_eq!(
            files.len(),
            2,
            "regular subdirectories should be walked, got {} files",
            files.len()
        );
    }
}
