//! Typed identity types and ID generation.
//!
//! All entities in Spectron use strongly-typed IDs to prevent accidental mixing
//! of identifiers across different entity kinds (crates, modules, symbols, files).
//! The [`IdGenerator`] provides thread-safe monotonically increasing ID allocation.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

/// Unique identifier for a crate within a project.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct CrateId(pub u64);

/// Unique identifier for a module within a project.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct ModuleId(pub u64);

/// Unique identifier for a symbol (function, struct, trait, etc.).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct SymbolId(pub u64);

/// Unique identifier for a source file.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub struct FileId(pub u64);

impl fmt::Display for CrateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CrateId({})", self.0)
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ModuleId({})", self.0)
    }
}

impl fmt::Display for SymbolId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SymbolId({})", self.0)
    }
}

impl fmt::Display for FileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FileId({})", self.0)
    }
}

/// Thread-safe generator of monotonically increasing IDs.
///
/// A single `IdGenerator` instance uses one shared atomic counter, so all ID
/// types share the same sequence space. This guarantees global uniqueness across
/// all entity kinds within a single generator instance.
///
/// # Examples
///
/// ```
/// use spectron_core::id::IdGenerator;
///
/// let gen = IdGenerator::new();
/// let c1 = gen.next_crate();
/// let m1 = gen.next_module();
/// assert_eq!(c1.0, 0);
/// assert_eq!(m1.0, 1);
/// ```
pub struct IdGenerator {
    next: AtomicU64,
}

impl IdGenerator {
    /// Create a new generator starting at 0.
    pub fn new() -> Self {
        Self {
            next: AtomicU64::new(0),
        }
    }

    /// Generate the next unique [`CrateId`].
    pub fn next_crate(&self) -> CrateId {
        CrateId(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Generate the next unique [`ModuleId`].
    pub fn next_module(&self) -> ModuleId {
        ModuleId(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Generate the next unique [`SymbolId`].
    pub fn next_symbol(&self) -> SymbolId {
        SymbolId(self.next.fetch_add(1, Ordering::Relaxed))
    }

    /// Generate the next unique [`FileId`].
    pub fn next_file(&self) -> FileId {
        FileId(self.next.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for IdGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn crate_ids_are_monotonically_increasing() {
        let gen = IdGenerator::new();
        let a = gen.next_crate();
        let b = gen.next_crate();
        let c = gen.next_crate();
        assert!(a.0 < b.0, "expected {} < {}", a.0, b.0);
        assert!(b.0 < c.0, "expected {} < {}", b.0, c.0);
    }

    #[test]
    fn module_ids_are_monotonically_increasing() {
        let gen = IdGenerator::new();
        let a = gen.next_module();
        let b = gen.next_module();
        let c = gen.next_module();
        assert!(a.0 < b.0);
        assert!(b.0 < c.0);
    }

    #[test]
    fn symbol_ids_are_monotonically_increasing() {
        let gen = IdGenerator::new();
        let a = gen.next_symbol();
        let b = gen.next_symbol();
        let c = gen.next_symbol();
        assert!(a.0 < b.0);
        assert!(b.0 < c.0);
    }

    #[test]
    fn file_ids_are_monotonically_increasing() {
        let gen = IdGenerator::new();
        let a = gen.next_file();
        let b = gen.next_file();
        let c = gen.next_file();
        assert!(a.0 < b.0);
        assert!(b.0 < c.0);
    }

    #[test]
    fn mixed_id_types_share_sequence_and_are_globally_unique() {
        let gen = IdGenerator::new();
        let c = gen.next_crate();
        let m = gen.next_module();
        let s = gen.next_symbol();
        let f = gen.next_file();

        // All raw values should be distinct and increasing
        assert_eq!(c.0, 0);
        assert_eq!(m.0, 1);
        assert_eq!(s.0, 2);
        assert_eq!(f.0, 3);
    }

    #[test]
    fn id_equality_works() {
        let a = CrateId(42);
        let b = CrateId(42);
        let c = CrateId(99);

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn id_copy_semantics() {
        let a = SymbolId(10);
        let b = a; // Copy
        assert_eq!(a, b); // a is still valid
    }

    #[test]
    fn ids_usable_as_hash_keys() {
        let mut set = HashSet::new();
        set.insert(ModuleId(1));
        set.insert(ModuleId(2));
        set.insert(ModuleId(1)); // duplicate

        assert_eq!(set.len(), 2);
    }

    #[test]
    fn display_formatting() {
        assert_eq!(format!("{}", CrateId(5)), "CrateId(5)");
        assert_eq!(format!("{}", ModuleId(10)), "ModuleId(10)");
        assert_eq!(format!("{}", SymbolId(42)), "SymbolId(42)");
        assert_eq!(format!("{}", FileId(0)), "FileId(0)");
    }

    #[test]
    fn serde_roundtrip() {
        let id = CrateId(123);
        let json = serde_json::to_string(&id).expect("serialize failed");
        let deserialized: CrateId = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(id, deserialized);

        let id = ModuleId(456);
        let json = serde_json::to_string(&id).expect("serialize failed");
        let deserialized: ModuleId = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(id, deserialized);

        let id = SymbolId(789);
        let json = serde_json::to_string(&id).expect("serialize failed");
        let deserialized: SymbolId = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(id, deserialized);

        let id = FileId(0);
        let json = serde_json::to_string(&id).expect("serialize failed");
        let deserialized: FileId = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(id, deserialized);
    }

    /// Compile-time type safety test: different ID types cannot be mixed.
    ///
    /// The following code must NOT compile. We verify this by demonstrating that
    /// each ID type is accepted only where its own type is expected.
    /// If someone were to write `let _: CrateId = gen.next_module();` it would
    /// be a compile error because ModuleId != CrateId.
    #[test]
    fn type_safety_ids_are_distinct_types() {
        fn accept_crate(_id: CrateId) {}
        fn accept_module(_id: ModuleId) {}
        fn accept_symbol(_id: SymbolId) {}
        fn accept_file(_id: FileId) {}

        let gen = IdGenerator::new();

        // Each generator method returns the correct type
        accept_crate(gen.next_crate());
        accept_module(gen.next_module());
        accept_symbol(gen.next_symbol());
        accept_file(gen.next_file());

        // These would fail to compile if uncommented:
        // accept_crate(gen.next_module());  // error: expected CrateId, found ModuleId
        // accept_module(gen.next_symbol()); // error: expected ModuleId, found SymbolId
        // accept_symbol(gen.next_file());   // error: expected SymbolId, found FileId
        // accept_file(gen.next_crate());    // error: expected FileId, found CrateId
    }

    #[test]
    fn id_generator_default_trait() {
        let gen = IdGenerator::default();
        let id = gen.next_crate();
        assert_eq!(id.0, 0);
    }

    #[test]
    fn concurrent_id_generation() {
        use std::sync::Arc;
        use std::thread;

        let gen = Arc::new(IdGenerator::new());
        let mut handles = Vec::new();

        for _ in 0..4 {
            let gen = Arc::clone(&gen);
            handles.push(thread::spawn(move || {
                let mut ids = Vec::new();
                for _ in 0..100 {
                    ids.push(gen.next_symbol().0);
                }
                ids
            }));
        }

        let mut all_ids: Vec<u64> = Vec::new();
        for handle in handles {
            all_ids.extend(handle.join().expect("thread panicked"));
        }

        // All 400 IDs should be unique
        let unique: HashSet<u64> = all_ids.iter().copied().collect();
        assert_eq!(unique.len(), 400, "expected 400 unique IDs, got {}", unique.len());

        // All IDs should be in range [0, 400)
        assert_eq!(*all_ids.iter().min().unwrap(), 0);
        assert_eq!(*all_ids.iter().max().unwrap(), 399);
    }
}
