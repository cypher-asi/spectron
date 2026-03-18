//! Project hierarchy types: ProjectInfo, CrateInfo, ModuleInfo, and related types.
//!
//! These types model the structural hierarchy of a Rust project:
//! `ProjectInfo` -> `CrateInfo` -> `ModuleInfo`.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::id::{CrateId, ModuleId, SymbolId};
use crate::traits::Labeled;

// ---------------------------------------------------------------------------
// ModulePath
// ---------------------------------------------------------------------------

/// Fully qualified module path, e.g. `"my_crate::foo::bar"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModulePath(pub String);

impl ModulePath {
    /// Create a new module path from a string.
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }

    /// Return the path segments split by `::`.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split("::")
    }

    /// Return the underlying string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ModulePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for ModulePath {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ModulePath {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

// ---------------------------------------------------------------------------
// CrateType
// ---------------------------------------------------------------------------

/// The kind of a crate (binary or library).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrateType {
    Binary,
    Library,
}

// ---------------------------------------------------------------------------
// ProjectInfo
// ---------------------------------------------------------------------------

/// Root-level project metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectInfo {
    /// Display name (from Cargo.toml or directory name).
    pub name: String,
    /// Absolute path to the project root.
    pub root_path: PathBuf,
    /// Whether this is a Cargo workspace.
    pub is_workspace: bool,
    /// Crate IDs belonging to this project.
    pub crate_ids: Vec<CrateId>,
}

impl ProjectInfo {
    /// Create a new `ProjectInfo`.
    pub fn new(name: impl Into<String>, root_path: impl Into<PathBuf>, is_workspace: bool) -> Self {
        Self {
            name: name.into(),
            root_path: root_path.into(),
            is_workspace,
            crate_ids: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// CrateInfo
// ---------------------------------------------------------------------------

/// Metadata for a single crate within the project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrateInfo {
    /// Unique identifier for this crate.
    pub id: CrateId,
    /// Crate name (as declared in Cargo.toml).
    pub name: String,
    /// Path to the crate root directory.
    pub path: PathBuf,
    /// Whether this is a binary or library crate.
    pub crate_type: CrateType,
    /// Module IDs belonging to this crate.
    pub module_ids: Vec<ModuleId>,
    /// Names of crate dependencies from `[dependencies]` in Cargo.toml.
    ///
    /// These are the crate names as they appear in the manifest (using
    /// underscores, matching Rust's crate naming convention). External
    /// dependencies that are not part of the project are included here
    /// but will not resolve to graph nodes during DependsOn edge creation.
    pub dependencies: Vec<String>,
}

impl CrateInfo {
    /// Create a new `CrateInfo`.
    pub fn new(
        id: CrateId,
        name: impl Into<String>,
        path: impl Into<PathBuf>,
        crate_type: CrateType,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            path: path.into(),
            crate_type,
            module_ids: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}

impl Labeled for CrateInfo {
    fn label(&self) -> &str {
        &self.name
    }

    fn qualified_label(&self) -> String {
        self.name.clone()
    }
}

// ---------------------------------------------------------------------------
// ModuleInfo
// ---------------------------------------------------------------------------

/// Metadata for a single module within a crate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModuleInfo {
    /// Unique identifier for this module.
    pub id: ModuleId,
    /// Short module name (e.g. `"bar"` for `my_crate::foo::bar`).
    pub name: String,
    /// Fully qualified path (e.g. `"my_crate::foo::bar"`).
    pub path: ModulePath,
    /// Source file backing this module, if known.
    pub file_path: Option<PathBuf>,
    /// Parent module, or `None` for the crate root module.
    pub parent: Option<ModuleId>,
    /// Direct child modules.
    pub children: Vec<ModuleId>,
    /// Symbols declared in this module.
    pub symbol_ids: Vec<SymbolId>,
}

impl ModuleInfo {
    /// Create a new `ModuleInfo` with empty children and symbols.
    pub fn new(
        id: ModuleId,
        name: impl Into<String>,
        path: ModulePath,
        file_path: Option<PathBuf>,
        parent: Option<ModuleId>,
    ) -> Self {
        Self {
            id,
            name: name.into(),
            path,
            file_path,
            parent,
            children: Vec::new(),
            symbol_ids: Vec::new(),
        }
    }
}

impl Labeled for ModuleInfo {
    fn label(&self) -> &str {
        &self.name
    }

    fn qualified_label(&self) -> String {
        self.path.0.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdGenerator;

    #[test]
    fn project_info_construction() {
        let project = ProjectInfo::new("my-project", "/home/user/my-project", true);
        assert_eq!(project.name, "my-project");
        assert_eq!(project.root_path, PathBuf::from("/home/user/my-project"));
        assert!(project.is_workspace);
        assert!(project.crate_ids.is_empty());
    }

    #[test]
    fn crate_info_construction() {
        let gen = IdGenerator::new();
        let id = gen.next_crate();
        let info = CrateInfo::new(id, "my-crate", "/home/user/my-crate", CrateType::Library);
        assert_eq!(info.id, id);
        assert_eq!(info.name, "my-crate");
        assert_eq!(info.crate_type, CrateType::Library);
        assert!(info.module_ids.is_empty());
    }

    #[test]
    fn module_info_construction() {
        let gen = IdGenerator::new();
        let mid = gen.next_module();
        let info = ModuleInfo::new(
            mid,
            "bar",
            ModulePath::new("my_crate::foo::bar"),
            Some(PathBuf::from("src/foo/bar.rs")),
            None,
        );
        assert_eq!(info.id, mid);
        assert_eq!(info.name, "bar");
        assert_eq!(info.path.as_str(), "my_crate::foo::bar");
        assert_eq!(info.file_path, Some(PathBuf::from("src/foo/bar.rs")));
        assert!(info.parent.is_none());
        assert!(info.children.is_empty());
        assert!(info.symbol_ids.is_empty());
    }

    #[test]
    fn module_info_with_parent() {
        let gen = IdGenerator::new();
        let parent_id = gen.next_module();
        let child_id = gen.next_module();
        let child = ModuleInfo::new(
            child_id,
            "child",
            ModulePath::new("my_crate::parent::child"),
            None,
            Some(parent_id),
        );
        assert_eq!(child.parent, Some(parent_id));
    }

    #[test]
    fn module_path_display() {
        let path = ModulePath::new("my_crate::foo::bar");
        assert_eq!(format!("{}", path), "my_crate::foo::bar");
    }

    #[test]
    fn module_path_segments() {
        let path = ModulePath::new("my_crate::foo::bar");
        let segments: Vec<&str> = path.segments().collect();
        assert_eq!(segments, vec!["my_crate", "foo", "bar"]);
    }

    #[test]
    fn module_path_from_str() {
        let path: ModulePath = "my_crate::foo".into();
        assert_eq!(path.as_str(), "my_crate::foo");
    }

    #[test]
    fn module_path_from_string() {
        let path: ModulePath = String::from("my_crate::foo").into();
        assert_eq!(path.as_str(), "my_crate::foo");
    }

    #[test]
    fn crate_type_variants() {
        let bin = CrateType::Binary;
        let lib = CrateType::Library;
        assert_ne!(bin, lib);
        assert_eq!(bin, CrateType::Binary);
        assert_eq!(lib, CrateType::Library);
    }

    #[test]
    fn project_info_crate_ids_mutable() {
        let gen = IdGenerator::new();
        let mut project = ProjectInfo::new("proj", "/tmp/proj", false);
        let c1 = gen.next_crate();
        let c2 = gen.next_crate();
        project.crate_ids.push(c1);
        project.crate_ids.push(c2);
        assert_eq!(project.crate_ids.len(), 2);
        assert_eq!(project.crate_ids[0], c1);
        assert_eq!(project.crate_ids[1], c2);
    }

    #[test]
    fn crate_info_module_ids_mutable() {
        let gen = IdGenerator::new();
        let cid = gen.next_crate();
        let mut info = CrateInfo::new(cid, "lib", "/tmp/lib", CrateType::Library);
        let m1 = gen.next_module();
        let m2 = gen.next_module();
        info.module_ids.push(m1);
        info.module_ids.push(m2);
        assert_eq!(info.module_ids.len(), 2);
    }

    #[test]
    fn module_info_children_and_symbols_mutable() {
        let gen = IdGenerator::new();
        let mid = gen.next_module();
        let mut info = ModuleInfo::new(mid, "root", ModulePath::new("my_crate"), None, None);

        let child = gen.next_module();
        info.children.push(child);
        assert_eq!(info.children.len(), 1);

        let sym = gen.next_symbol();
        info.symbol_ids.push(sym);
        assert_eq!(info.symbol_ids.len(), 1);
    }

    #[test]
    fn serde_roundtrip_project_info() {
        let gen = IdGenerator::new();
        let mut project = ProjectInfo::new("test-proj", "/tmp/test", true);
        project.crate_ids.push(gen.next_crate());

        let json = serde_json::to_string(&project).expect("serialize failed");
        let deser: ProjectInfo = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(deser.name, project.name);
        assert_eq!(deser.root_path, project.root_path);
        assert_eq!(deser.is_workspace, project.is_workspace);
        assert_eq!(deser.crate_ids, project.crate_ids);
    }

    #[test]
    fn serde_roundtrip_crate_info() {
        let gen = IdGenerator::new();
        let id = gen.next_crate();
        let mut info = CrateInfo::new(id, "serde-test", "/tmp/c", CrateType::Binary);
        info.module_ids.push(gen.next_module());

        let json = serde_json::to_string(&info).expect("serialize failed");
        let deser: CrateInfo = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(deser.id, info.id);
        assert_eq!(deser.name, info.name);
        assert_eq!(deser.crate_type, info.crate_type);
        assert_eq!(deser.module_ids, info.module_ids);
    }

    #[test]
    fn serde_roundtrip_module_info() {
        let gen = IdGenerator::new();
        let mid = gen.next_module();
        let parent = gen.next_module();
        let mut info = ModuleInfo::new(
            mid,
            "child",
            ModulePath::new("crate::child"),
            Some(PathBuf::from("src/child.rs")),
            Some(parent),
        );
        info.children.push(gen.next_module());
        info.symbol_ids.push(gen.next_symbol());

        let json = serde_json::to_string(&info).expect("serialize failed");
        let deser: ModuleInfo = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(deser.id, info.id);
        assert_eq!(deser.name, info.name);
        assert_eq!(deser.path, info.path);
        assert_eq!(deser.file_path, info.file_path);
        assert_eq!(deser.parent, info.parent);
        assert_eq!(deser.children, info.children);
        assert_eq!(deser.symbol_ids, info.symbol_ids);
    }

    // -----------------------------------------------------------------------
    // Labeled trait tests
    // -----------------------------------------------------------------------

    #[test]
    fn crate_info_labeled_label() {
        let gen = IdGenerator::new();
        let info = CrateInfo::new(
            gen.next_crate(),
            "my-crate",
            "/tmp/my-crate",
            CrateType::Library,
        );
        assert_eq!(info.label(), "my-crate");
    }

    #[test]
    fn crate_info_labeled_qualified_label() {
        let gen = IdGenerator::new();
        let info = CrateInfo::new(
            gen.next_crate(),
            "my-crate",
            "/tmp/my-crate",
            CrateType::Binary,
        );
        // For a crate, the qualified label is the crate name itself.
        assert_eq!(info.qualified_label(), "my-crate");
    }

    #[test]
    fn module_info_labeled_label() {
        let gen = IdGenerator::new();
        let info = ModuleInfo::new(
            gen.next_module(),
            "bar",
            ModulePath::new("my_crate::foo::bar"),
            None,
            None,
        );
        assert_eq!(info.label(), "bar");
    }

    #[test]
    fn module_info_labeled_qualified_label() {
        let gen = IdGenerator::new();
        let info = ModuleInfo::new(
            gen.next_module(),
            "bar",
            ModulePath::new("my_crate::foo::bar"),
            None,
            None,
        );
        assert_eq!(info.qualified_label(), "my_crate::foo::bar");
    }

    #[test]
    fn module_info_labeled_root_module() {
        let gen = IdGenerator::new();
        let info = ModuleInfo::new(
            gen.next_module(),
            "my_crate",
            ModulePath::new("my_crate"),
            None,
            None,
        );
        assert_eq!(info.label(), "my_crate");
        assert_eq!(info.qualified_label(), "my_crate");
    }
}
