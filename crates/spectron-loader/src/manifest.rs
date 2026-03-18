//! Cargo.toml manifest parsing and workspace detection.
//!
//! This module reads a `Cargo.toml` file and determines whether it represents
//! a workspace (with member globs) or a single crate (with a `[package]` section).

use std::path::{Path, PathBuf};

use spectron_core::error::SpectronError;

/// The kind of manifest discovered after parsing a `Cargo.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ManifestKind {
    /// A workspace root with resolved member directories.
    Workspace {
        /// Display name derived from the workspace directory or the first package name.
        name: String,
        /// Resolved absolute paths to each workspace member directory.
        members: Vec<PathBuf>,
    },
    /// A single crate with a `[package]` section.
    SingleCrate {
        /// Package name from `[package].name`.
        name: String,
    },
}

/// Parse a `Cargo.toml` at the given path and determine its kind.
///
/// If the manifest contains a `[workspace]` section with `members`, it is
/// treated as a workspace root and the member glob patterns are resolved
/// against the filesystem.
///
/// If no `[workspace]` section exists, the manifest is treated as a single
/// crate and the `[package].name` is extracted.
///
/// # Errors
///
/// Returns [`SpectronError::NoCargo`] if the file does not exist, or
/// [`SpectronError::Parse`] if the TOML cannot be parsed or is malformed.
pub fn parse_manifest(manifest_path: &Path) -> Result<ManifestKind, SpectronError> {
    let content = std::fs::read_to_string(manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            SpectronError::NoCargo {
                path: manifest_path
                    .parent()
                    .unwrap_or(manifest_path)
                    .to_path_buf(),
            }
        } else {
            SpectronError::Io(e)
        }
    })?;

    parse_manifest_str(&content, manifest_path)
}

/// Parse a `Cargo.toml` from its string content.
///
/// `manifest_path` is used for error messages and for resolving workspace
/// member globs relative to the manifest's parent directory.
fn parse_manifest_str(
    content: &str,
    manifest_path: &Path,
) -> Result<ManifestKind, SpectronError> {
    let manifest = cargo_toml::Manifest::from_str(content).map_err(|e| SpectronError::Parse {
        file: manifest_path.to_path_buf(),
        message: format!("failed to parse Cargo.toml: {}", e),
    })?;

    // Check for workspace section first.
    if let Some(workspace) = &manifest.workspace {
        let workspace_dir = manifest_path
            .parent()
            .unwrap_or(Path::new("."));

        let members = resolve_member_globs(&workspace.members, workspace_dir);

        // Derive the workspace name: use the package name if this is also a
        // package (common in virtual workspaces that are also packages), or
        // fall back to the directory name.
        let name = manifest
            .package
            .as_ref()
            .map(|p| p.name.clone())
            .unwrap_or_else(|| {
                workspace_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("workspace")
                    .to_string()
            });

        return Ok(ManifestKind::Workspace { name, members });
    }

    // No workspace -- must be a single crate with a [package] section.
    let package = manifest.package.as_ref().ok_or_else(|| SpectronError::Parse {
        file: manifest_path.to_path_buf(),
        message: "Cargo.toml has neither [workspace] nor [package] section".to_string(),
    })?;

    Ok(ManifestKind::SingleCrate {
        name: package.name.clone(),
    })
}

/// Extract dependency crate names from a parsed `Cargo.toml`.
///
/// Returns the names of all crates listed under `[dependencies]`,
/// `[dev-dependencies]`, and `[build-dependencies]`. Only direct
/// dependencies are included (not transitive). The names are returned
/// as they appear in the manifest key (e.g. `serde`, `my-crate`).
///
/// Returns an empty vec if the manifest cannot be read or parsed.
pub fn extract_dependency_names(manifest_path: &Path) -> Vec<String> {
    let content = match std::fs::read_to_string(manifest_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(
                path = %manifest_path.display(),
                error = %e,
                "could not read Cargo.toml for dependency extraction"
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
                "could not parse Cargo.toml for dependency extraction"
            );
            return Vec::new();
        }
    };

    extract_dep_names_from_manifest(&manifest)
}

/// Extract dependency names from an already-parsed manifest.
///
/// Includes `[dependencies]`, `[dev-dependencies]`, and
/// `[build-dependencies]`. Deduplicates across sections.
fn extract_dep_names_from_manifest(manifest: &cargo_toml::Manifest) -> Vec<String> {
    use std::collections::BTreeSet;

    let mut names = BTreeSet::new();

    for key in manifest.dependencies.keys() {
        names.insert(key.clone());
    }
    for key in manifest.dev_dependencies.keys() {
        names.insert(key.clone());
    }
    for key in manifest.build_dependencies.keys() {
        names.insert(key.clone());
    }

    names.into_iter().collect()
}

/// Resolve workspace member glob patterns to actual directories on disk.
///
/// Each pattern in `members` is interpreted as a glob relative to `workspace_dir`.
/// Patterns that match no directories are logged as warnings.
/// Non-existent or non-directory matches are skipped with warnings.
fn resolve_member_globs(members: &[String], workspace_dir: &Path) -> Vec<PathBuf> {
    let mut resolved = Vec::new();

    for pattern in members {
        let full_pattern = workspace_dir.join(pattern);
        let pattern_str = match full_pattern.to_str() {
            Some(s) => s.to_string(),
            None => {
                tracing::warn!(
                    pattern = pattern.as_str(),
                    "workspace member pattern is not valid UTF-8, skipping"
                );
                continue;
            }
        };

        match glob::glob(&pattern_str) {
            Ok(paths) => {
                let mut matched = false;
                for entry in paths {
                    match entry {
                        Ok(path) => {
                            if path.is_dir() {
                                resolved.push(path);
                                matched = true;
                            } else {
                                tracing::warn!(
                                    path = %path.display(),
                                    "workspace member glob matched a non-directory, skipping"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "error while resolving workspace member glob"
                            );
                        }
                    }
                }
                if !matched {
                    tracing::warn!(
                        pattern = pattern.as_str(),
                        "workspace member pattern matched no directories"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    pattern = pattern.as_str(),
                    error = %e,
                    "invalid workspace member glob pattern"
                );
            }
        }
    }

    resolved
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: parse a TOML string as if it came from the given path.
    fn parse_str(content: &str) -> Result<ManifestKind, SpectronError> {
        parse_manifest_str(content, Path::new("Cargo.toml"))
    }

    // -----------------------------------------------------------------------
    // Workspace detection
    // -----------------------------------------------------------------------

    #[test]
    fn parse_workspace_manifest_extracts_members() {
        // Create a temp directory with member subdirectories so glob resolution works.
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let workspace_dir = tmp.path();

        // Create member directories.
        fs::create_dir_all(workspace_dir.join("crates/alpha/src")).unwrap();
        fs::create_dir_all(workspace_dir.join("crates/beta/src")).unwrap();

        // Write the workspace Cargo.toml.
        let manifest_content = r#"
[workspace]
members = ["crates/alpha", "crates/beta"]
resolver = "2"
"#;
        let manifest_path = workspace_dir.join("Cargo.toml");
        fs::write(&manifest_path, manifest_content).unwrap();

        let result = parse_manifest(&manifest_path).expect("parse should succeed");

        match result {
            ManifestKind::Workspace { name, members } => {
                // Name should fall back to directory name since there is no [package].
                assert!(!name.is_empty(), "workspace name should not be empty");

                assert_eq!(members.len(), 2, "expected 2 workspace members");
                let member_names: Vec<String> = members
                    .iter()
                    .map(|p| p.file_name().unwrap().to_str().unwrap().to_string())
                    .collect();
                assert!(
                    member_names.contains(&"alpha".to_string()),
                    "should contain alpha, got: {:?}",
                    member_names
                );
                assert!(
                    member_names.contains(&"beta".to_string()),
                    "should contain beta, got: {:?}",
                    member_names
                );
            }
            ManifestKind::SingleCrate { .. } => {
                panic!("expected Workspace, got SingleCrate");
            }
        }
    }

    #[test]
    fn parse_workspace_with_glob_patterns() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let workspace_dir = tmp.path();

        // Create member directories matching a glob.
        fs::create_dir_all(workspace_dir.join("crates/foo")).unwrap();
        fs::create_dir_all(workspace_dir.join("crates/bar")).unwrap();

        let manifest_content = r#"
[workspace]
members = ["crates/*"]
"#;
        let manifest_path = workspace_dir.join("Cargo.toml");
        fs::write(&manifest_path, manifest_content).unwrap();

        let result = parse_manifest(&manifest_path).expect("parse should succeed");

        match result {
            ManifestKind::Workspace { members, .. } => {
                assert_eq!(members.len(), 2, "expected 2 members from glob");
            }
            _ => panic!("expected Workspace"),
        }
    }

    #[test]
    fn parse_workspace_with_package_uses_package_name() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let workspace_dir = tmp.path();

        let manifest_content = r#"
[workspace]
members = []

[package]
name = "my-workspace-root"
version = "0.1.0"
edition = "2021"
"#;
        let manifest_path = workspace_dir.join("Cargo.toml");
        fs::write(&manifest_path, manifest_content).unwrap();

        let result = parse_manifest(&manifest_path).expect("parse should succeed");

        match result {
            ManifestKind::Workspace { name, .. } => {
                assert_eq!(name, "my-workspace-root");
            }
            _ => panic!("expected Workspace"),
        }
    }

    // -----------------------------------------------------------------------
    // Single crate detection
    // -----------------------------------------------------------------------

    #[test]
    fn parse_single_crate_manifest_extracts_name() {
        let content = r#"
[package]
name = "my-awesome-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
"#;

        let result = parse_str(content).expect("parse should succeed");

        match result {
            ManifestKind::SingleCrate { name } => {
                assert_eq!(name, "my-awesome-crate");
            }
            ManifestKind::Workspace { .. } => {
                panic!("expected SingleCrate, got Workspace");
            }
        }
    }

    #[test]
    fn parse_single_crate_with_bin_sections() {
        let content = r#"
[package]
name = "my-cli"
version = "1.0.0"
edition = "2021"

[[bin]]
name = "my-cli"
path = "src/main.rs"
"#;

        let result = parse_str(content).expect("parse should succeed");

        match result {
            ManifestKind::SingleCrate { name } => {
                assert_eq!(name, "my-cli");
            }
            _ => panic!("expected SingleCrate"),
        }
    }

    // -----------------------------------------------------------------------
    // Error cases
    // -----------------------------------------------------------------------

    #[test]
    fn parse_manifest_returns_no_cargo_for_missing_file() {
        let result = parse_manifest(Path::new("/nonexistent/path/Cargo.toml"));
        assert!(result.is_err(), "should return error for missing file");

        match result.unwrap_err() {
            SpectronError::NoCargo { path } => {
                assert_eq!(path, PathBuf::from("/nonexistent/path"));
            }
            other => panic!("expected NoCargo, got: {:?}", other),
        }
    }

    #[test]
    fn parse_malformed_toml_returns_parse_error() {
        let content = r#"
[package
name = "broken"
"#;

        let result = parse_str(content);
        assert!(result.is_err(), "should return error for malformed TOML");

        match result.unwrap_err() {
            SpectronError::Parse { file, message } => {
                assert_eq!(file, PathBuf::from("Cargo.toml"));
                assert!(
                    message.contains("failed to parse"),
                    "unexpected error message: {}",
                    message
                );
            }
            other => panic!("expected Parse error, got: {:?}", other),
        }
    }

    #[test]
    fn parse_toml_without_workspace_or_package_returns_parse_error() {
        let content = r#"
[dependencies]
serde = "1"
"#;

        let result = parse_str(content);
        assert!(result.is_err(), "should return error for missing sections");

        match result.unwrap_err() {
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
    fn parse_workspace_with_missing_member_dirs_returns_empty_members() {
        let tmp = tempfile::tempdir().expect("failed to create temp dir");
        let workspace_dir = tmp.path();

        // No member directories created on disk.
        let manifest_content = r#"
[workspace]
members = ["nonexistent-crate"]
"#;
        let manifest_path = workspace_dir.join("Cargo.toml");
        fs::write(&manifest_path, manifest_content).unwrap();

        let result = parse_manifest(&manifest_path).expect("parse should succeed");

        match result {
            ManifestKind::Workspace { members, .. } => {
                assert!(
                    members.is_empty(),
                    "expected empty members for missing dirs, got: {:?}",
                    members
                );
            }
            _ => panic!("expected Workspace"),
        }
    }

    // -----------------------------------------------------------------------
    // Dependency extraction
    // -----------------------------------------------------------------------

    #[test]
    fn extract_dep_names_from_dependencies_section() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
tokio = { version = "1", features = ["full"] }
"#,
        )
        .unwrap();

        let names = extract_dep_names_from_manifest(&manifest);
        assert!(names.contains(&"serde".to_owned()), "should contain serde");
        assert!(names.contains(&"tokio".to_owned()), "should contain tokio");
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn extract_dep_names_includes_dev_and_build_deps() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"

[dev-dependencies]
tempfile = "3"

[build-dependencies]
cc = "1"
"#,
        )
        .unwrap();

        let names = extract_dep_names_from_manifest(&manifest);
        assert!(names.contains(&"serde".to_owned()), "should contain serde");
        assert!(names.contains(&"tempfile".to_owned()), "should contain tempfile");
        assert!(names.contains(&"cc".to_owned()), "should contain cc");
        assert_eq!(names.len(), 3);
    }

    #[test]
    fn extract_dep_names_deduplicates_across_sections() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"

[dev-dependencies]
serde = { version = "1", features = ["derive"] }
"#,
        )
        .unwrap();

        let names = extract_dep_names_from_manifest(&manifest);
        assert_eq!(names.len(), 1, "serde should appear only once");
        assert_eq!(names[0], "serde");
    }

    #[test]
    fn extract_dep_names_empty_when_no_dependencies() {
        let manifest = cargo_toml::Manifest::from_str(
            r#"[package]
name = "my-crate"
version = "0.1.0"
edition = "2021"
"#,
        )
        .unwrap();

        let names = extract_dep_names_from_manifest(&manifest);
        assert!(names.is_empty(), "should be empty when no dependencies");
    }

    #[test]
    fn extract_dependency_names_from_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let manifest_path = tmp.path().join("Cargo.toml");
        fs::write(
            &manifest_path,
            r#"[package]
name = "file-test"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = "1"
petgraph = "0.6"
"#,
        )
        .unwrap();

        let names = extract_dependency_names(&manifest_path);
        assert_eq!(names.len(), 2);
        assert!(names.contains(&"petgraph".to_owned()));
        assert!(names.contains(&"serde".to_owned()));
    }

    #[test]
    fn extract_dependency_names_returns_empty_for_missing_file() {
        let names = extract_dependency_names(Path::new("/nonexistent/Cargo.toml"));
        assert!(names.is_empty());
    }
}
