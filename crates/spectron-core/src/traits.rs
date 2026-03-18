//! Shared trait definitions used across the Spectron system.

use crate::id::FileId;
use crate::symbol::SourceSpan;

/// Anything that can be displayed as a graph node label.
pub trait Labeled {
    /// A short display label (typically the entity name).
    fn label(&self) -> &str;

    /// A fully qualified label (e.g. `"my_crate::foo::bar::MyStruct"`).
    fn qualified_label(&self) -> String;
}

/// Anything that occupies a position in source code.
pub trait Spanned {
    /// The source span of this entity.
    fn span(&self) -> &SourceSpan;

    /// The file in which this entity is located.
    fn file_id(&self) -> FileId;
}
