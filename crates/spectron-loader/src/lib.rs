//! spectron-loader: workspace, crate, and module discovery.
//!
//! This crate takes a filesystem path pointing to a Rust project and discovers
//! its structure: workspace membership, crate metadata, module trees, and source
//! files. It does **not** parse Rust source code -- only discovers project layout.

mod crates;
mod files;
mod manifest;
mod modules;

use std::path::Path;

use spectron_core::error::SpectronError;
use spectron_core::{CrateInfo, FileInfo, ModuleInfo, ProjectInfo};

pub use crates::discover_crate_targets;
pub use files::discover_files;
pub use manifest::{extract_dependency_names, parse_manifest, ManifestKind};
pub use modules::discover_modules;

/// The result of loading a project's structure from disk.
#[derive(Debug)]
pub struct LoadResult {
    /// Root project metadata.
    pub project: ProjectInfo,
    /// All discovered crates within the project.
    pub crates: Vec<CrateInfo>,
    /// All discovered modules across all crates.
    pub modules: Vec<ModuleInfo>,
    /// All discovered source files across all crates.
    pub files: Vec<FileInfo>,
}

/// Entry point: load project structure from a directory.
///
/// The given `path` should point to a directory containing a `Cargo.toml`.
/// Returns [`SpectronError::NoCargo`] if no manifest is found.
pub fn load_project(path: &Path) -> Result<LoadResult, SpectronError> {
    let manifest_path = path.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(SpectronError::NoCargo {
            path: path.to_path_buf(),
        });
    }

    let manifest_kind = parse_manifest(&manifest_path)?;

    let id_gen = spectron_core::IdGenerator::new();

    match manifest_kind {
        ManifestKind::Workspace { name, members } => {
            let mut project = ProjectInfo::new(&name, path, true);
            let mut crates = Vec::new();

            for member_path in &members {
                let member_manifest_path = member_path.join("Cargo.toml");
                if !member_manifest_path.exists() {
                    tracing::warn!(
                        path = %member_path.display(),
                        "workspace member directory has no Cargo.toml, skipping"
                    );
                    continue;
                }

                let member_kind = match parse_manifest(&member_manifest_path) {
                    Ok(kind) => kind,
                    Err(e) => {
                        tracing::warn!(
                            path = %member_manifest_path.display(),
                            error = %e,
                            "failed to parse workspace member Cargo.toml, skipping"
                        );
                        continue;
                    }
                };

                if let ManifestKind::SingleCrate { name: crate_name } = member_kind {
                    let discovered =
                        discover_crate_targets(&id_gen, &crate_name, member_path);
                    for ci in &discovered {
                        project.crate_ids.push(ci.id);
                    }
                    crates.extend(discovered);
                }
            }

            let files = discover_files_for_crates(&id_gen, &crates);
            let modules = discover_modules_for_crates(&id_gen, &mut crates);

            Ok(LoadResult {
                project,
                crates,
                modules,
                files,
            })
        }
        ManifestKind::SingleCrate { name: crate_name } => {
            let mut project = ProjectInfo::new(&crate_name, path, false);
            let mut crates = discover_crate_targets(&id_gen, &crate_name, path);
            for ci in &crates {
                project.crate_ids.push(ci.id);
            }

            let files = discover_files_for_crates(&id_gen, &crates);
            let modules = discover_modules_for_crates(&id_gen, &mut crates);

            Ok(LoadResult {
                project,
                crates,
                modules,
                files,
            })
        }
    }
}

/// Discover source files for all crates, deduplicating by path.
///
/// When a crate has both library and binary targets, they share the same
/// `src/` directory. We only walk each unique crate path once to avoid
/// duplicate [`FileInfo`] entries.
fn discover_files_for_crates(
    id_gen: &spectron_core::IdGenerator,
    crates: &[CrateInfo],
) -> Vec<FileInfo> {
    let mut seen_paths = std::collections::HashSet::new();
    let mut all_files = Vec::new();

    for krate in crates {
        if seen_paths.insert(krate.path.clone()) {
            let files = discover_files(id_gen, &krate.path);
            all_files.extend(files);
        }
    }

    all_files
}

/// Discover modules for all crate targets and populate their `module_ids`.
///
/// Returns a flat list of all [`ModuleInfo`] entries across all crates.
/// Each crate's `module_ids` field is updated with the IDs of its modules.
///
/// ## Deduplication for dual-target crates
///
/// When a crate has both a library and binary target (both `lib.rs` and
/// `main.rs`), they share the same `src/` directory. If both targets declare
/// the same child modules (e.g. `mod shared;` in both files), those modules
/// resolve to the same source file.
///
/// To avoid duplicate `ModuleInfo` entries for the same source file within the
/// same crate directory, this function builds a lookup of already-discovered
/// modules keyed by `(crate_path, file_path)`. When a second target discovers
/// a module whose file has already been processed for the same crate path, the
/// existing module's ID is reused (added to the second target's `module_ids`)
/// rather than creating a duplicate entry.
///
/// Root modules (`lib.rs` / `main.rs`) are always distinct since they represent
/// different compilation entry points.
fn discover_modules_for_crates(
    id_gen: &spectron_core::IdGenerator,
    crates: &mut [CrateInfo],
) -> Vec<ModuleInfo> {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use spectron_core::id::ModuleId;

    // Map from (crate_path, module_file_path) -> ModuleId for deduplication.
    // This allows the second target of a dual-target crate to reference
    // modules already discovered by the first target.
    let mut file_to_module: HashMap<(PathBuf, PathBuf), ModuleId> = HashMap::new();

    let mut all_modules = Vec::new();

    for krate in crates.iter_mut() {
        let modules = discover_modules(id_gen, krate);

        // First pass: build a remap table for this target. Maps newly
        // generated module IDs to existing IDs when a non-root module
        // resolves to a file already discovered for the same crate path.
        let mut remap: HashMap<ModuleId, ModuleId> = HashMap::new();

        for m in &modules {
            if let Some(ref file_path) = m.file_path {
                let key = (krate.path.clone(), file_path.clone());
                if let Some(&existing_id) = file_to_module.get(&key) {
                    // Non-root modules that resolve to an already-discovered
                    // file are deduplicated. Root modules (parent == None) are
                    // always distinct because lib.rs and main.rs are different
                    // entry points.
                    if m.parent.is_some() {
                        remap.insert(m.id, existing_id);
                    }
                }
            }
        }

        // Second pass: add modules, applying the remap to children vectors
        // so that parent modules reference the deduplicated child IDs.
        for m in modules {
            if let Some(&existing_id) = remap.get(&m.id) {
                // This module is a duplicate; reference the existing one.
                krate.module_ids.push(existing_id);
                continue;
            }

            let mut module = m;

            // Rewrite children IDs: if any child was deduplicated, point to
            // the existing module ID instead of the now-skipped duplicate.
            for child_id in &mut module.children {
                if let Some(&remapped) = remap.get(child_id) {
                    *child_id = remapped;
                }
            }

            // Register this module's file for future deduplication.
            if let Some(ref file_path) = module.file_path {
                let key = (krate.path.clone(), file_path.clone());
                file_to_module.entry(key).or_insert(module.id);
            }

            krate.module_ids.push(module.id);
            all_modules.push(module);
        }
    }

    all_modules
}
