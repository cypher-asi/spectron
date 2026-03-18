//! Security indicator types for flagging potentially sensitive code patterns.
//!
//! These types are used to track unsafe code, FFI boundaries, filesystem access,
//! network access, and subprocess execution within the analyzed codebase.

use serde::{Deserialize, Serialize};

use crate::id::SymbolId;
use crate::symbol::SourceSpan;

// ---------------------------------------------------------------------------
// SecurityIndicator
// ---------------------------------------------------------------------------

/// A security-relevant code pattern detected during analysis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum SecurityIndicator {
    /// An `unsafe { }` block in the source code.
    UnsafeBlock {
        /// Location of the unsafe block.
        span: SourceSpan,
    },
    /// A function declared as `unsafe fn`.
    UnsafeFunction {
        /// The symbol ID of the unsafe function.
        symbol_id: SymbolId,
    },
    /// A call across an FFI boundary.
    FfiCall {
        /// Location of the FFI call.
        span: SourceSpan,
        /// Name of the extern function being called.
        extern_name: String,
    },
    /// Access to the filesystem (e.g. `std::fs::read`, `File::open`).
    FilesystemAccess {
        /// Location of the filesystem access call.
        span: SourceSpan,
        /// Name of the function performing filesystem access.
        function_name: String,
    },
    /// Network access (e.g. `TcpStream::connect`).
    NetworkAccess {
        /// Location of the network access call.
        span: SourceSpan,
        /// Name of the function performing network access.
        function_name: String,
    },
    /// Subprocess execution (e.g. `Command::new`).
    SubprocessExecution {
        /// Location of the subprocess execution call.
        span: SourceSpan,
        /// Name of the function executing a subprocess.
        function_name: String,
    },
}

// ---------------------------------------------------------------------------
// SecurityReport
// ---------------------------------------------------------------------------

/// Aggregated security indicators for an entire analysis pass.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SecurityReport {
    /// All security indicators found during analysis.
    pub indicators: Vec<SecurityIndicator>,
}

impl SecurityReport {
    /// Create an empty security report.
    pub fn new() -> Self {
        Self {
            indicators: Vec::new(),
        }
    }
}

impl Default for SecurityReport {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::id::FileId;

    #[test]
    fn security_report_new_is_empty() {
        let report = SecurityReport::new();
        assert!(report.indicators.is_empty());
    }

    #[test]
    fn security_report_default_is_empty() {
        let report = SecurityReport::default();
        assert!(report.indicators.is_empty());
    }

    #[test]
    fn unsafe_block_indicator() {
        let span = SourceSpan::new(FileId(0), 10, 4, 10, 30);
        let indicator = SecurityIndicator::UnsafeBlock { span: span.clone() };

        if let SecurityIndicator::UnsafeBlock { span: s } = &indicator {
            assert_eq!(s, &span);
        } else {
            panic!("expected UnsafeBlock variant");
        }
    }

    #[test]
    fn unsafe_function_indicator() {
        let indicator = SecurityIndicator::UnsafeFunction {
            symbol_id: SymbolId(42),
        };

        if let SecurityIndicator::UnsafeFunction { symbol_id } = &indicator {
            assert_eq!(*symbol_id, SymbolId(42));
        } else {
            panic!("expected UnsafeFunction variant");
        }
    }

    #[test]
    fn ffi_call_indicator() {
        let span = SourceSpan::new(FileId(1), 20, 0, 20, 50);
        let indicator = SecurityIndicator::FfiCall {
            span: span.clone(),
            extern_name: "sqlite3_open".to_owned(),
        };

        if let SecurityIndicator::FfiCall { span: s, extern_name } = &indicator {
            assert_eq!(s, &span);
            assert_eq!(extern_name, "sqlite3_open");
        } else {
            panic!("expected FfiCall variant");
        }
    }

    #[test]
    fn filesystem_access_indicator() {
        let span = SourceSpan::new(FileId(2), 5, 0, 5, 25);
        let indicator = SecurityIndicator::FilesystemAccess {
            span: span.clone(),
            function_name: "std::fs::read".to_owned(),
        };

        if let SecurityIndicator::FilesystemAccess { span: s, function_name } = &indicator {
            assert_eq!(s, &span);
            assert_eq!(function_name, "std::fs::read");
        } else {
            panic!("expected FilesystemAccess variant");
        }
    }

    #[test]
    fn network_access_indicator() {
        let span = SourceSpan::new(FileId(3), 15, 0, 15, 40);
        let indicator = SecurityIndicator::NetworkAccess {
            span: span.clone(),
            function_name: "TcpStream::connect".to_owned(),
        };

        if let SecurityIndicator::NetworkAccess { span: s, function_name } = &indicator {
            assert_eq!(s, &span);
            assert_eq!(function_name, "TcpStream::connect");
        } else {
            panic!("expected NetworkAccess variant");
        }
    }

    #[test]
    fn subprocess_execution_indicator() {
        let span = SourceSpan::new(FileId(4), 30, 0, 30, 35);
        let indicator = SecurityIndicator::SubprocessExecution {
            span: span.clone(),
            function_name: "Command::new".to_owned(),
        };

        if let SecurityIndicator::SubprocessExecution { span: s, function_name } = &indicator {
            assert_eq!(s, &span);
            assert_eq!(function_name, "Command::new");
        } else {
            panic!("expected SubprocessExecution variant");
        }
    }

    #[test]
    fn security_report_add_indicators() {
        let mut report = SecurityReport::new();
        report.indicators.push(SecurityIndicator::UnsafeBlock {
            span: SourceSpan::new(FileId(0), 1, 0, 1, 10),
        });
        report.indicators.push(SecurityIndicator::UnsafeFunction {
            symbol_id: SymbolId(1),
        });
        assert_eq!(report.indicators.len(), 2);
    }

    #[test]
    fn security_indicator_clone() {
        let indicator = SecurityIndicator::FfiCall {
            span: SourceSpan::new(FileId(0), 1, 0, 1, 10),
            extern_name: "test_fn".to_owned(),
        };
        let cloned = indicator.clone();

        if let (
            SecurityIndicator::FfiCall { extern_name: a, .. },
            SecurityIndicator::FfiCall { extern_name: b, .. },
        ) = (&indicator, &cloned)
        {
            assert_eq!(a, b);
        } else {
            panic!("expected FfiCall variants");
        }
    }

    #[test]
    fn serde_roundtrip_security_indicator() {
        let indicator = SecurityIndicator::FfiCall {
            span: SourceSpan::new(FileId(5), 10, 2, 10, 40),
            extern_name: "libc_call".to_owned(),
        };
        let json = serde_json::to_string(&indicator).expect("serialize failed");
        let deser: SecurityIndicator = serde_json::from_str(&json).expect("deserialize failed");

        if let SecurityIndicator::FfiCall { extern_name, .. } = &deser {
            assert_eq!(extern_name, "libc_call");
        } else {
            panic!("expected FfiCall variant after deserialization");
        }
    }

    #[test]
    fn serde_roundtrip_security_report() {
        let mut report = SecurityReport::new();
        report.indicators.push(SecurityIndicator::UnsafeBlock {
            span: SourceSpan::new(FileId(0), 1, 0, 3, 1),
        });
        report.indicators.push(SecurityIndicator::NetworkAccess {
            span: SourceSpan::new(FileId(1), 20, 0, 20, 50),
            function_name: "connect".to_owned(),
        });

        let json = serde_json::to_string(&report).expect("serialize failed");
        let deser: SecurityReport = serde_json::from_str(&json).expect("deserialize failed");
        assert_eq!(deser.indicators.len(), 2);
    }
}
