//! Error types for the Spectron system.
//!
//! [`SpectronError`] is the top-level error type shared across all Spectron
//! crates. Each variant covers a distinct failure domain (IO, parsing,
//! project discovery, storage, rendering).

use std::path::PathBuf;

/// Top-level error type for the Spectron system.
#[derive(Debug, thiserror::Error)]
pub enum SpectronError {
    /// An underlying IO operation failed.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// A source file could not be parsed.
    #[error("Parse error in {file}: {message}")]
    Parse {
        /// Path to the file that failed to parse.
        file: PathBuf,
        /// Human-readable description of the parse failure.
        message: String,
    },

    /// No `Cargo.toml` was found at the expected location.
    #[error("No Cargo.toml found in {path}")]
    NoCargo {
        /// Directory that was searched for a `Cargo.toml`.
        path: PathBuf,
    },

    /// A storage / persistence layer error.
    #[error("Storage error: {0}")]
    Storage(String),

    /// A rendering / GPU error.
    #[error("Render error: {0}")]
    Render(String),
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn io_error_display() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "file not found");
        let err = SpectronError::from(io_err);
        let msg = format!("{}", err);
        assert!(msg.starts_with("IO error: "), "unexpected message: {}", msg);
        assert!(msg.contains("file not found"), "unexpected message: {}", msg);
    }

    #[test]
    fn io_error_from_conversion() {
        let io_err = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let err: SpectronError = io_err.into();
        assert!(matches!(err, SpectronError::Io(_)));
    }

    #[test]
    fn parse_error_display() {
        let err = SpectronError::Parse {
            file: PathBuf::from("src/main.rs"),
            message: "unexpected token".to_owned(),
        };
        let msg = format!("{}", err);
        assert_eq!(msg, "Parse error in src/main.rs: unexpected token");
    }

    #[test]
    fn no_cargo_error_display() {
        let err = SpectronError::NoCargo {
            path: PathBuf::from("/home/user/project"),
        };
        let msg = format!("{}", err);
        assert_eq!(msg, "No Cargo.toml found in /home/user/project");
    }

    #[test]
    fn storage_error_display() {
        let err = SpectronError::Storage("database connection failed".to_owned());
        let msg = format!("{}", err);
        assert_eq!(msg, "Storage error: database connection failed");
    }

    #[test]
    fn render_error_display() {
        let err = SpectronError::Render("shader compilation failed".to_owned());
        let msg = format!("{}", err);
        assert_eq!(msg, "Render error: shader compilation failed");
    }

    #[test]
    fn error_is_send_and_sync() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<SpectronError>();
        assert_sync::<SpectronError>();
    }

    #[test]
    fn error_debug_output() {
        let err = SpectronError::Storage("test".to_owned());
        let debug = format!("{:?}", err);
        assert!(debug.contains("Storage"), "unexpected debug: {}", debug);
    }
}
