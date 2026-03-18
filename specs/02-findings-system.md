# 02 -- Unified Findings & Evidence System

## PRD Reference

Every finding must include:

```
- ID
- Severity
- Confidence
- Category (security / perf / architecture)
- Location
- Call path (if applicable)
- Explanation
- Suggested fix
- Reachability (public / internal)
```

The PRD envisions a single evidence engine that all analysis modules emit into,
producing consistent, explainable, actionable findings.

---

## Current Implementation

There is no unified findings type. Three separate ad-hoc types serve as
finding-like outputs:

### 1. SecurityIndicator (`spectron-core/src/security.rs`)

```rust
pub enum SecurityIndicator {
    UnsafeBlock { span: SourceSpan },
    UnsafeFunction { symbol_id: SymbolId },
    FfiCall { span, extern_name },
    FilesystemAccess { span, function_name },
    NetworkAccess { span, function_name },
    SubprocessExecution { span, function_name },
}
```

Aggregated into `SecurityReport { indicators: Vec<SecurityIndicator> }`.

**Limitations**: No severity, no confidence, no explanation, no suggested fix,
no category taxonomy. Just raw pattern detections.

### 2. ComplexityFlag (`spectron-analysis/src/types.rs`)

```rust
pub struct ComplexityFlag {
    pub target: FlagTarget,          // Symbol(SymbolId) | Module(ModuleId)
    pub kind: ComplexityFlagKind,    // HighCyclomaticComplexity, LargeFunction, LargeModule, HighFanIn, HighFanOut
    pub value: u32,
    pub threshold: u32,
}
```

**Limitations**: No severity (all flags are implicitly "warning"), no confidence,
no explanation text, no suggested fix.

### 3. ParseError (`spectron-core/src/analysis.rs`)

```rust
pub struct ParseError {
    pub file_path: PathBuf,
    pub message: String,
    pub span: Option<SourceSpan>,
}
```

Represents parse failures, not analysis findings. Should remain separate
but be convertible to a Finding for unified reporting.

### Current Aggregation

`AnalysisOutput` (in `spectron-analysis/src/types.rs`) holds:

```rust
pub struct AnalysisOutput {
    pub symbol_metrics: HashMap<SymbolId, SymbolMetrics>,
    pub module_metrics: HashMap<ModuleId, ModuleMetrics>,
    pub security_report: SecurityReport,
    pub entrypoints: Vec<SymbolId>,
    pub complexity_flags: Vec<ComplexityFlag>,
}
```

Each consumer (UI, CLI, JSON) must inspect these separate collections individually.

---

## Gap Analysis

| PRD Field | SecurityIndicator | ComplexityFlag | ParseError |
|---|---|---|---|
| ID | None | None | None |
| Severity | None | Implicit "warning" | Implicit "error" |
| Confidence | None | None (always 100%) | N/A |
| Category | Implicit "security" | Implicit "quality" | Implicit "parse" |
| Location | `span` (some variants) | `target` (symbol/module) | `file_path` + `span` |
| Call path | None | None | N/A |
| Explanation | None | None | `message` |
| Suggested fix | None | None | None |
| Reachability | None | None | N/A |

**Key gap**: No single type unifies all analysis outputs. Each new analysis
module would add yet another ad-hoc type without a shared schema.

---

## Design: Unified Finding Type

### Location in Codebase

Add `finding.rs` to `spectron-core/src/` since Finding is a domain type
consumed by all downstream crates (analysis, UI, CLI, storage).

### Proposed Data Model

```rust
pub struct Finding {
    pub id: FindingId,
    pub rule_id: String,
    pub severity: Severity,
    pub confidence: Confidence,
    pub category: FindingCategory,
    pub location: FindingLocation,
    pub call_path: Option<Vec<SymbolId>>,
    pub title: String,
    pub explanation: String,
    pub suggested_fix: Option<String>,
    pub reachability: Reachability,
    pub metadata: HashMap<String, String>,
}

pub struct FindingId(pub u64);

pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

pub enum Confidence {
    High,    // 90-100%
    Medium,  // 60-89%
    Low,     // <60%
}

pub enum FindingCategory {
    Security,
    Performance,
    Architecture,
    Quality,
    Resource,
}

pub struct FindingLocation {
    pub file_id: Option<FileId>,
    pub file_path: Option<PathBuf>,
    pub span: Option<SourceSpan>,
    pub symbol_id: Option<SymbolId>,
    pub module_id: Option<ModuleId>,
}

pub enum Reachability {
    Public,
    Internal,
    Unknown,
}
```

### Integration Strategy

1. **Existing types remain as-is**: `SecurityIndicator`, `ComplexityFlag`, and
   `ParseError` continue to be produced by their respective modules.

2. **Conversion layer**: Each module provides a `fn into_findings(...) -> Vec<Finding>`
   that converts its native outputs into unified findings. This avoids breaking
   existing code while enabling unified reporting.

3. **AnalysisOutput gains a findings vec**:

```rust
pub struct AnalysisOutput {
    // ... existing fields ...
    pub findings: Vec<Finding>,
}
```

4. **New analysis modules emit findings directly**: Taint engine, auth analyzer,
   etc. produce `Vec<Finding>` from the start, not custom types.

### Rule ID Convention

Format: `<category>/<module>/<check-name>`

Examples:
- `security/taint/sql-injection`
- `security/auth/missing-guard`
- `perf/cost/db-in-loop`
- `arch/structure/cyclic-dependency`
- `quality/complexity/high-cyclomatic`

---

## Design Decisions

1. **Keep existing types**: `SecurityIndicator` and `ComplexityFlag` are retained
   for backwards compatibility and because they carry module-specific detail.
   The `Finding` is the _output_ format, not the internal analysis representation.

2. **FindingId from IdGenerator**: Reuse the existing `IdGenerator` by adding
   a `next_finding()` method, keeping global ID uniqueness.

3. **Metadata as HashMap**: The `metadata` field allows modules to attach
   arbitrary key-value pairs (e.g., `"sink_function": "execute_sql"`) without
   requiring Finding to know about every module's internals.

4. **Reachability computed lazily**: Reachability requires call-graph analysis
   from entrypoints. Rather than mandating it at finding creation time, default
   to `Unknown` and let a post-processing pass fill it in.

5. **Explanation is mandatory**: Every finding must have a human-readable
   explanation. This enforces the PRD's "no black box" principle.

---

## Tasks

- [ ] **T-02.1** Create `spectron-core/src/finding.rs` with `Finding`, `FindingId`, `Severity`, `Confidence`, `FindingCategory`, `FindingLocation`, `Reachability`
- [ ] **T-02.2** Add `FindingId` to `IdGenerator` (`next_finding()`)
- [ ] **T-02.3** Re-export finding types from `spectron-core/src/lib.rs`
- [ ] **T-02.4** Add `findings: Vec<Finding>` field to `AnalysisOutput`
- [ ] **T-02.5** Implement `SecurityIndicator -> Vec<Finding>` conversion in `spectron-analysis/src/security.rs`
- [ ] **T-02.6** Implement `ComplexityFlag -> Vec<Finding>` conversion in `spectron-analysis/src/metrics.rs`
- [ ] **T-02.7** Implement `ParseError -> Vec<Finding>` conversion (severity=Info, category=Quality)
- [ ] **T-02.8** Wire finding conversions into `analyze()` so `AnalysisOutput.findings` is populated
- [ ] **T-02.9** Add `--findings` flag to CLI that prints findings as JSON
- [ ] **T-02.10** Add serde derives to all Finding types for JSON serialization
- [ ] **T-02.11** Add unit tests for Finding construction and conversion from existing types
